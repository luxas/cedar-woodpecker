#!/usr/bin/env bash
# Implicit permission: create Deployment -> create ReplicaSet
#   (deployment-create-creates-replicaset)
#
# Proof: an identity granted ONLY `create deployments` cannot `create
# replicasets`, yet after it creates a Deployment the deployment controller
# creates a ReplicaSet (owned by the Deployment) on its behalf.
# Spec: ../transitions/deployment-create-creates-replicaset.json
#   p = create apps::deployments ; q = create apps::replicasets ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "deployment-create-creates-replicaset"

ns="$(new_ns deploy-rs)"
mk_sa "$ns" attacker
# ONLY deployments (+read replicasets to observe). Deliberately NO replicasets write.
grant_ns "$ns" attacker "apps" "deployments" "create,get,list"
grant_ns "$ns" attacker "apps" "replicasets" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create deployments "$ns"
assert_cannot "$subj" create replicasets "$ns"

step "attacker creates a Deployment (replicas=1)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the deployment"
apiVersion: apps/v1
kind: Deployment
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

# EFFECT: a ReplicaSet the attacker could not create directly appears, owned by
# the Deployment, created by the deployment controller on the attacker's behalf.
wait_exists "$ns" replicasets "app=stamp" 60
owner="$(K -n "$ns" get rs -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "Deployment" "replicaset is controller-owned by the Deployment"
step "deployment controller created a $owner-owned ReplicaSet from a Deployment the attacker made with only 'create deployments'"

test_pass
