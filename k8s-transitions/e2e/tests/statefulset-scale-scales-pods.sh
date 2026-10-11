#!/usr/bin/env bash
# Implicit permission: update statefulsets/scale -> create Pod
#   (statefulset-scale-scales-pods)
#
# Proof: an identity granted ONLY `update/patch statefulsets/scale` cannot
# `create pods` and cannot write the parent StatefulSet, yet writing /scale
# moves spec.replicas and the StatefulSet controller stamps new Pods on its
# behalf (ordinal 0..N-1).
# Mechanism: ../transitions/scale-subresource-scales-pods.md
#            ../transitions/statefulset-scale-scales-pods.json
source "$(dirname "$0")/../lib.sh"
test_start "statefulset-scale-scales-pods"

ns="$(new_ns sts-scale)"

# StatefulSet requires a governing (headless) Service; admin creates both.
step "admin creates a headless Service + StatefulSet at replicas=1"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create statefulset"
apiVersion: v1
kind: Service
metadata: { name: target, labels: { app: target } }
spec:
  clusterIP: None
  selector: { app: target }
  ports: [ { port: 80, name: p } ]
---
apiVersion: apps/v1
kind: StatefulSet
metadata: { name: target, labels: { app: target } }
spec:
  serviceName: target
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
# StatefulSet creates Pods one ordinal at a time; wait for the first to be Ready
# so later ordinals are not blocked behind an unschedulable pod-0.
wait_pod_running "$ns" target-0 90
one="$(K -n "$ns" get pods -l app=target --no-headers | wc -l)"
assert_eq "$one" "1" "statefulset starts at exactly one Pod"

mk_sa "$ns" attacker
# ONLY the scale subresource (+read to observe). NO pods write, NO parent write.
# p is update/patch on /scale; `kubectl scale` issues a PATCH, so grant both.
grant_ns "$ns" attacker "apps" "statefulsets/scale" "update,patch,get"
grant_ns "$ns" attacker "apps" "statefulsets" "get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" patch  statefulsets/scale "$ns"   # sanity: holds p
assert_cannot "$subj" create pods               "$ns"   # THE GAP
assert_cannot "$subj" patch  statefulsets       "$ns"   # cannot touch the parent spec
assert_cannot "$subj" update statefulsets       "$ns"   # cannot touch the parent spec

step "attacker scales the StatefulSet 1 -> 3 via the /scale subresource"
as_subj "$subj" -n "$ns" scale statefulset/target --replicas=3 >/dev/null \
  || fail "attacker could not scale via /scale subresource"

step "waiting for the Pod count to converge to 3"
converged=0
for _ in $(seq 1 150); do
  n="$(K -n "$ns" get pods -l app=target --no-headers 2>/dev/null | wc -l)"
  [ "${n:-0}" -ge 3 ] && { converged=1; break; }
  # unblock ordered creation: ensure each existing ordinal becomes Ready
  K -n "$ns" wait --for=condition=Ready pod -l app=target --timeout=5s >/dev/null 2>&1 || true
  sleep 1
done
[ "$converged" = 1 ] || fail "pods did not reach 3 (saw $(K -n "$ns" get pods -l app=target --no-headers | wc -l): $(K -n "$ns" get pods -l app=target --no-headers 2>/dev/null | awk '{print $1,$3}' | tr '\n' ';'))"
owner="$(K -n "$ns" get pods -l app=target -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "StatefulSet" "scaled Pods are controller-owned, not attacker-authored"
step "the StatefulSet controller created extra $owner-owned Pods from a /scale write with only 'update statefulsets/scale'"

test_pass
