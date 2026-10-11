#!/usr/bin/env bash
# Implicit permission: create ReplicaSet -> create Pod   (replicaset-create-creates-pod)
#
# Proof: an identity granted ONLY `create replicasets` cannot `create pods`,
# yet after it creates a ReplicaSet the controller-manager stamps a Pod from the
# template on its behalf (controller-template mechanism).
# Spec: ../transitions/replicaset-create-creates-pod.json
#   p = create apps::replicasets ; q = create core::pods ; when same namespace.
source "$(dirname "$0")/../lib.sh"
test_start "replicaset-create-creates-pod"

ns="$(new_ns rs-pod)"
mk_sa "$ns" attacker
# ONLY replicasets (+read to observe). Deliberately NO pods write.
grant_ns "$ns" attacker "apps" "replicasets" "create,get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create replicasets "$ns"
assert_cannot "$subj" create pods        "$ns"

step "attacker creates a ReplicaSet (replicas=1)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the replicaset"
apiVersion: apps/v1
kind: ReplicaSet
metadata: { name: stamp, labels: { app: stamp } }
spec:
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

# EFFECT: a Pod the attacker could not create directly appears, owned by the RS.
wait_exists "$ns" pods "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "ReplicaSet" "pod is controller-owned, not attacker-authored"
step "controller-manager stamped a $owner-owned Pod from a ReplicaSet the attacker made with only 'create replicasets'"

test_pass
