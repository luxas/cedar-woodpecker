#!/usr/bin/env bash
# Implicit permission: create StatefulSet -> create ControllerRevision
#   (statefulset-create-creates-controllerrevision)
#
# Proof: an identity granted ONLY `create statefulsets` cannot `create
# controllerrevisions`, yet after it creates a StatefulSet the statefulset
# controller writes a ControllerRevision (template history, owned by the STS).
# Spec: ../transitions/statefulset-create-creates-controllerrevision.json
#   p = create apps::statefulsets ; q = create apps::controllerrevisions ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "statefulset-create-creates-controllerrevision"

ns="$(new_ns sts-cr)"
mk_sa "$ns" attacker
# ONLY statefulsets (+read controllerrevisions to observe). NO controllerrevisions write.
grant_ns "$ns" attacker "apps" "statefulsets" "create,get,list"
grant_ns "$ns" attacker "apps" "controllerrevisions" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create statefulsets          "$ns"
assert_cannot "$subj" create controllerrevisions   "$ns"

step "attacker creates a StatefulSet (replicas=1)"
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

# EFFECT: a ControllerRevision the attacker could not create directly appears,
# owned by the StatefulSet (controller records template history on its behalf).
stsuid="$(K -n "$ns" get sts stamp -o jsonpath='{.metadata.uid}')"
i=0; crname=""
while [ $i -lt 60 ]; do
  crname="$(K -n "$ns" get controllerrevisions \
            -o jsonpath="{range .items[?(@.metadata.ownerReferences[0].uid=='$stsuid')]}{.metadata.name}{'\n'}{end}" 2>/dev/null | head -1)"
  [ -n "$crname" ] && break
  sleep 1; i=$((i+1))
done
assert_nonempty "$crname" "a ControllerRevision owned by the StatefulSet appeared"
owner="$(K -n "$ns" get controllerrevision "$crname" -o jsonpath='{.metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "StatefulSet" "controllerrevision is owned by the StatefulSet"
step "statefulset controller created ControllerRevision '$crname' ($owner-owned) from a StatefulSet the attacker made with only 'create statefulsets'"

test_pass
