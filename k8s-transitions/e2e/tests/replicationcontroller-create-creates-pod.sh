#!/usr/bin/env bash
# Implicit permission: create ReplicationController -> create Pod
#   (replicationcontroller-create-creates-pod)
#
# Proof: an identity granted ONLY `create replicationcontrollers` cannot
# `create pods`, yet after it creates an RC the controller-manager stamps a Pod
# from the template on its behalf.
# Spec: ../transitions/replicationcontroller-create-creates-pod.json
#   p = create core::replicationcontrollers ; q = create core::pods ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "replicationcontroller-create-creates-pod"

ns="$(new_ns rc-pod)"
mk_sa "$ns" attacker
# ONLY replicationcontrollers (+read to observe). Deliberately NO pods write.
grant_ns "$ns" attacker "" "replicationcontrollers" "create,get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create replicationcontrollers "$ns"
assert_cannot "$subj" create pods                   "$ns"

step "attacker creates a ReplicationController (replicas=1)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the replicationcontroller"
apiVersion: v1
kind: ReplicationController
metadata: { name: stamp, labels: { app: stamp } }
spec:
  replicas: 1
  selector: { app: stamp }
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

# EFFECT: a Pod the attacker could not create directly appears, owned by the RC.
wait_exists "$ns" pods "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "ReplicationController" "pod is controller-owned, not attacker-authored"
step "controller-manager stamped a $owner-owned Pod from an RC the attacker made with only 'create replicationcontrollers'"

test_pass
