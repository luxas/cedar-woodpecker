#!/usr/bin/env bash
# Implicit permission: create DaemonSet -> create ControllerRevision
#   (daemonset-create-creates-controllerrevision)
#
# Proof: an identity granted ONLY `create daemonsets` cannot `create
# controllerrevisions`, yet after it creates a DaemonSet the daemonset
# controller writes a ControllerRevision (history of the pod template, owned by
# the DaemonSet) on its behalf.
# Spec: ../transitions/daemonset-create-creates-controllerrevision.json
#   p = create apps::daemonsets ; q = create apps::controllerrevisions ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "daemonset-create-creates-controllerrevision"

ns="$(new_ns ds-cr)"
mk_sa "$ns" attacker
# ONLY daemonsets (+read controllerrevisions to observe). NO controllerrevisions write.
grant_ns "$ns" attacker "apps" "daemonsets" "create,get,list"
grant_ns "$ns" attacker "apps" "controllerrevisions" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create daemonsets           "$ns"
assert_cannot "$subj" create controllerrevisions  "$ns"

step "attacker creates a DaemonSet"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the daemonset"
apiVersion: apps/v1
kind: DaemonSet
metadata: { name: stamp, labels: { app: stamp } }
spec:
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
# owned by the DaemonSet (the controller records template history on its behalf).
dsuid="$(K -n "$ns" get ds stamp -o jsonpath='{.metadata.uid}')"
i=0; crname=""
while [ $i -lt 60 ]; do
  crname="$(K -n "$ns" get controllerrevisions \
            -o jsonpath="{range .items[?(@.metadata.ownerReferences[0].uid=='$dsuid')]}{.metadata.name}{'\n'}{end}" 2>/dev/null | head -1)"
  [ -n "$crname" ] && break
  sleep 1; i=$((i+1))
done
assert_nonempty "$crname" "a ControllerRevision owned by the DaemonSet appeared"
owner="$(K -n "$ns" get controllerrevision "$crname" -o jsonpath='{.metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "DaemonSet" "controllerrevision is owned by the DaemonSet"
step "daemonset controller created ControllerRevision '$crname' ($owner-owned) from a DaemonSet the attacker made with only 'create daemonsets'"

test_pass
