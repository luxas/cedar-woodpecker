#!/usr/bin/env bash
# Implicit permission: create Deployment -> create Pod   (deployment-create-creates-pod)
#
# Proof: an identity granted ONLY `create deployments` cannot `create pods`,
# yet after it creates a Deployment the controller-manager stamps Pods from the
# template on its behalf. This is the controller-template mechanism underlying
# CVE-2024-43403 (a Pod template can reference Secrets/ServiceAccounts).
# Mechanism: ../transitions/deployment-manages-pods.md
source "$(dirname "$0")/../lib.sh"
test_start "deployment-create-creates-pod"

ns="$(new_ns deploy-pod)"
mk_sa "$ns" attacker
# ONLY deployments (+read to observe). Deliberately NO pods / replicasets write.
grant_ns "$ns" attacker "apps" "deployments" "create,get,list"
grant_ns "$ns" attacker "apps" "replicasets" "get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create deployments "$ns"
assert_cannot "$subj" create pods        "$ns"
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

# EFFECT: Pods (and a ReplicaSet) the attacker could not create directly appear,
# created by the controller-manager on the attacker's behalf.
wait_exists "$ns" replicasets "app=stamp" 60
wait_exists "$ns" pods        "app=stamp" 90
owner="$(K -n "$ns" get pods -l app=stamp -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "ReplicaSet" "pod is controller-owned, not attacker-authored"
step "controller-manager stamped a $owner-owned Pod from a Deployment the attacker made with only 'create deployments'"

test_pass
