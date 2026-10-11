#!/usr/bin/env bash
# Implicit permission: update replicasets/scale -> create Pod   (replicaset-scale-scales-pods)
#
# Proof: an identity granted ONLY `update/patch replicasets/scale` cannot
# `create pods` and cannot write the parent ReplicaSet, yet writing /scale moves
# spec.replicas and the ReplicaSet controller stamps new Pods on its behalf.
# Mechanism: ../transitions/scale-subresource-scales-pods.md
#            ../transitions/replicaset-scale-scales-pods.json
source "$(dirname "$0")/../lib.sh"
test_start "replicaset-scale-scales-pods"

ns="$(new_ns rs-scale)"

step "admin creates a ReplicaSet at replicas=1"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create replicaset"
apiVersion: apps/v1
kind: ReplicaSet
metadata: { name: target, labels: { app: target } }
spec:
  replicas: 1
  selector: { matchLabels: { app: target } }
  template:
    metadata: { labels: { app: target } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF
wait_exists "$ns" pods "app=target" 90
one="$(K -n "$ns" get pods -l app=target --no-headers | wc -l)"
assert_eq "$one" "1" "replicaset starts at exactly one Pod"

mk_sa "$ns" attacker
# ONLY the scale subresource (+read to observe). NO pods write, NO parent write.
# p is update/patch on /scale; `kubectl scale` issues a PATCH, so grant both.
grant_ns "$ns" attacker "apps" "replicasets/scale" "update,patch,get"
grant_ns "$ns" attacker "apps" "replicasets" "get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" patch  replicasets/scale "$ns"   # sanity: holds p
assert_cannot "$subj" create pods              "$ns"   # THE GAP
assert_cannot "$subj" patch  replicasets       "$ns"   # cannot touch the parent spec
assert_cannot "$subj" update replicasets       "$ns"   # cannot touch the parent spec

step "attacker scales the ReplicaSet 1 -> 3 via the /scale subresource"
as_subj "$subj" -n "$ns" scale replicaset/target --replicas=3 >/dev/null \
  || fail "attacker could not scale via /scale subresource"

step "waiting for the Pod count to converge to 3"
converged=0
for _ in $(seq 1 90); do
  n="$(K -n "$ns" get pods -l app=target --no-headers 2>/dev/null | wc -l)"
  [ "${n:-0}" -ge 3 ] && { converged=1; break; }
  sleep 1
done
[ "$converged" = 1 ] || fail "pods did not reach 3 (saw $(K -n "$ns" get pods -l app=target --no-headers | wc -l))"
owner="$(K -n "$ns" get pods -l app=target -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "ReplicaSet" "scaled Pods are controller-owned, not attacker-authored"
step "the ReplicaSet controller created extra $owner-owned Pods from a /scale write with only 'update replicasets/scale'"

test_pass
