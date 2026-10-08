#!/usr/bin/env bash
# Implicit permission: create Job -> create Pod  (job-create-creates-pod)
#
# Proof: an identity granted ONLY `create jobs` cannot `create pods`, yet after
# it creates a Job the job controller stamps a Pod from the template on its
# behalf (owned by the Job).
# Spec: ../transitions/job-create-creates-pod.json
#   p = create batch::jobs ; q = create core::pods ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "job-create-creates-pod"

ns="$(new_ns job-pod)"
mk_sa "$ns" attacker
# ONLY jobs (+read to observe). Deliberately NO pods write.
grant_ns "$ns" attacker "batch" "jobs" "create,get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create jobs "$ns"
assert_cannot "$subj" create pods "$ns"

step "attacker creates a Job (replicas=1 via single completion)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the job"
apiVersion: batch/v1
kind: Job
metadata: { name: stamp, labels: { app: stamp } }
spec:
  backoffLimit: 0
  template:
    metadata: { labels: { app: stamp } }
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF

# EFFECT: a Pod the attacker could not create directly appears, owned by the Job.
wait_exists "$ns" pods "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "Job" "pod is controller-owned, not attacker-authored"
step "job controller stamped a $owner-owned Pod from a Job the attacker made with only 'create jobs'"

test_pass
