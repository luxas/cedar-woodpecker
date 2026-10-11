#!/usr/bin/env bash
# Implicit permission: update pods/ephemeralcontainers -> update Pod
#                                            (pod-ephemeralcontainers-updates-pod)
#
# Proof: an identity granted ONLY `update pods/ephemeralcontainers` (plus the
# read on that subresource it needs to construct the PUT body) cannot `update
# pods`, yet PUTting the ephemeralcontainers subresource adds a debug container
# to the parent Pod -- the apiserver's EphemeralContainersREST handler writes
# the SAME stored Pod object (spec.ephemeralContainers grows) on its behalf.
# Mechanism: ../transitions/pod-ephemeralcontainers-updates-pod.md
#   p = update pods/ephemeralcontainers ; q = update pods ;
#   when = same name+namespace (subresource->parent identity match).
source "$(dirname "$0")/../lib.sh"
test_start "pod-ephemeralcontainers-updates-pod"

ns="$(new_ns pod-ec-upd)"

# Pre-existing state: a running Pod the attacker must not be able to update.
step "admin creates a running victim Pod"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create victim pod"
apiVersion: v1
kind: Pod
metadata: { name: victim }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" victim 90

mk_sa "$ns" attacker
# ONLY the ephemeralcontainers subresource: update to perform p, get to read the
# current subresource object so the PUT body is well-formed. Neither grants the
# parent `update pods` / `get pods`.
grant_ns "$ns" attacker "" "pods/ephemeralcontainers" "get,update"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" update pods/ephemeralcontainers "$ns"
assert_cannot "$subj" update pods                     "$ns"   # THE GAP

# ACT (as attacker, exercising ONLY p): read the subresource (which is the whole
# Pod), add an ephemeral/debug container, and PUT it back through the
# ephemeralcontainers subresource endpoint.
raw="/api/v1/namespaces/$ns/pods/victim/ephemeralcontainers"
step "attacker GETs the ephemeralcontainers subresource (returns the parent Pod)"
as_subj "$subj" get --raw "$raw" >/dev/null || fail "attacker could not read pods/ephemeralcontainers"

step "attacker PUTs the ephemeralcontainers subresource adding a debug container"
as_subj "$subj" replace --raw "$raw" -f - >/dev/null <<EOF \
  || fail "attacker could not update pods/ephemeralcontainers"
{"apiVersion":"v1","kind":"Pod",
 "metadata":{"name":"victim","namespace":"$ns"},
 "spec":{"ephemeralContainers":[
   {"name":"wp-dbg","image":"${IMG}","imagePullPolicy":"IfNotPresent",
    "command":["sh","-c","sleep 3600"],"terminationMessagePolicy":"File"}]}}
EOF

# EFFECT: the parent Pod's spec.ephemeralContainers grew -- a write to the Pod
# object the attacker could not perform via `update pods`. Observed as admin.
got="$(K -n "$ns" get pod victim -o jsonpath='{.spec.ephemeralContainers[*].name}')"
assert_contains "$got" "wp-dbg" "ephemeral container added to the parent Pod spec"
step "apiserver added ephemeral container '$got' to the parent Pod via a subresource PUT the attacker made with only 'update pods/ephemeralcontainers'"

test_pass
