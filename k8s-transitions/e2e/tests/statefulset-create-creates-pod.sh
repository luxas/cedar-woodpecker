#!/usr/bin/env bash
# Implicit permission: create StatefulSet -> create Pod  (statefulset-create-creates-pod)
#
# Proof: an identity granted ONLY `create statefulsets` cannot `create pods`,
# yet after it creates a StatefulSet the controller-manager stamps a Pod on its
# behalf (owned directly by the StatefulSet).
# Spec: ../transitions/statefulset-create-creates-pod.json
#   p = create apps::statefulsets ; q = create core::pods ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "statefulset-create-creates-pod"

ns="$(new_ns sts-pod)"
mk_sa "$ns" attacker
# ONLY statefulsets (+read to observe). Deliberately NO pods write.
grant_ns "$ns" attacker "apps" "statefulsets" "create,get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create statefulsets "$ns"
assert_cannot "$subj" create pods         "$ns"

step "attacker creates a StatefulSet (replicas=1, no volumeClaimTemplates)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the statefulset"
apiVersion: apps/v1
kind: StatefulSet
metadata: { name: stamp, labels: { app: stamp } }
spec:
  serviceName: stamp
  replicas: 1
  selector: { matchLabels: { app: stamp } }
  template:
    metadata: { labels: { app: stamp } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF

# EFFECT: a Pod the attacker could not create directly appears, owned by the STS.
wait_exists "$ns" pods "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "StatefulSet" "pod is controller-owned, not attacker-authored"
step "controller-manager stamped a $owner-owned Pod from a StatefulSet the attacker made with only 'create statefulsets'"

test_pass
