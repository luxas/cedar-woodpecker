#!/usr/bin/env bash
# Implicit permission: update replicationcontrollers/scale -> create Pod
#   (replicationcontroller-scale-scales-pods)
#
# Proof: an identity granted ONLY `update/patch replicationcontrollers/scale`
# cannot `create pods` and cannot write the parent RC, yet writing /scale moves
# spec.replicas and the RC controller stamps new Pods on its behalf.
# Mechanism: ../transitions/scale-subresource-scales-pods.md
#            ../transitions/replicationcontroller-scale-scales-pods.json
source "$(dirname "$0")/../lib.sh"
test_start "replicationcontroller-scale-scales-pods"

ns="$(new_ns rc-scale)"

step "admin creates a ReplicationController at replicas=1"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create replicationcontroller"
apiVersion: v1
kind: ReplicationController
metadata: { name: target, labels: { app: target } }
spec:
  replicas: 1
  selector: { app: target }
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
assert_eq "$one" "1" "replicationcontroller starts at exactly one Pod"

mk_sa "$ns" attacker
# ONLY the scale subresource (+read to observe). NO pods write, NO parent write.
# RC lives in the core ("") API group; `kubectl scale` issues a PATCH on /scale.
grant_ns "$ns" attacker "" "replicationcontrollers/scale" "update,patch,get"
grant_ns "$ns" attacker "" "replicationcontrollers" "get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" patch  replicationcontrollers/scale "$ns"   # sanity: holds p
assert_cannot "$subj" create pods                         "$ns"   # THE GAP
assert_cannot "$subj" patch  replicationcontrollers       "$ns"   # cannot touch the parent spec
assert_cannot "$subj" update replicationcontrollers       "$ns"   # cannot touch the parent spec

step "attacker scales the ReplicationController 1 -> 3 via the /scale subresource"
as_subj "$subj" -n "$ns" scale rc/target --replicas=3 >/dev/null \
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
assert_eq "$owner" "ReplicationController" "scaled Pods are controller-owned, not attacker-authored"
step "the RC controller created extra $owner-owned Pods from a /scale write with only 'update replicationcontrollers/scale'"

test_pass
