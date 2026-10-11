#!/usr/bin/env bash
# Implicit permission: create DaemonSet -> create Pod  (daemonset-create-creates-pod)
#
# Proof: an identity granted ONLY `create daemonsets` cannot `create pods`,
# yet after it creates a DaemonSet the daemonset controller stamps a Pod (one
# per eligible node; single-node kind => 1) on its behalf.
# Spec: ../transitions/daemonset-create-creates-pod.json
#   p = create apps::daemonsets ; q = create core::pods ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "daemonset-create-creates-pod"

ns="$(new_ns ds-pod)"
mk_sa "$ns" attacker
# ONLY daemonsets (+read to observe). Deliberately NO pods write.
grant_ns "$ns" attacker "apps" "daemonsets" "create,get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create daemonsets "$ns"
assert_cannot "$subj" create pods       "$ns"

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

# EFFECT: a Pod the attacker could not create directly appears, owned by the DS.
wait_exists "$ns" pods "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "DaemonSet" "pod is controller-owned, not attacker-authored"
step "daemonset controller stamped a $owner-owned Pod from a DaemonSet the attacker made with only 'create daemonsets'"

test_pass
