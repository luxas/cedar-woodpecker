#!/usr/bin/env bash
# Implicit permission: create CronJob -> create Job  (cronjob-create-creates-job)
#
# Proof: an identity granted ONLY `create cronjobs` cannot `create jobs`, yet
# once the schedule fires the cronjob controller creates a Job (owned by the
# CronJob) on its behalf. Schedule "* * * * *" => a Job within ~1 minute.
# Spec: ../transitions/cronjob-create-creates-job.json
#   p = create batch::cronjobs ; q = create batch::jobs ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "cronjob-create-creates-job"

ns="$(new_ns cj-job)"
mk_sa "$ns" attacker
# ONLY cronjobs (+read jobs to observe). Deliberately NO jobs write.
grant_ns "$ns" attacker "batch" "cronjobs" "create,get,list"
grant_ns "$ns" attacker "batch" "jobs" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create cronjobs "$ns"
assert_cannot "$subj" create jobs     "$ns"

step "attacker creates a CronJob (schedule '* * * * *')"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the cronjob"
apiVersion: batch/v1
kind: CronJob
metadata: { name: stamp, labels: { app: stamp } }
spec:
  schedule: "* * * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    metadata: { labels: { app: stamp } }
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
            command: ["sh","-c","true"]
EOF

# EFFECT: a Job the attacker could not create directly appears, owned by the
# CronJob (the cronjob controller creates it when the schedule fires).
cjuid="$(K -n "$ns" get cronjob stamp -o jsonpath='{.metadata.uid}')"
i=0; jobname=""
while [ $i -lt 100 ]; do
  jobname="$(K -n "$ns" get jobs \
             -o jsonpath="{range .items[?(@.metadata.ownerReferences[0].uid=='$cjuid')]}{.metadata.name}{'\n'}{end}" 2>/dev/null | head -1)"
  [ -n "$jobname" ] && break
  sleep 2; i=$((i+1))
done
assert_nonempty "$jobname" "a Job owned by the CronJob appeared (within ~200s)"
owner="$(K -n "$ns" get job "$jobname" -o jsonpath='{.metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "CronJob" "job is owned by the CronJob, not attacker-authored"
step "cronjob controller created Job '$jobname' ($owner-owned) from a CronJob the attacker made with only 'create cronjobs' -- attacker lacked 'create jobs'"

test_pass
