#!/usr/bin/env bash
# Implicit permission: get pods/ephemeralcontainers -> get Pod
#                                            (pods-ephemeralcontainers-reads-parent)
#
# Proof: an identity granted ONLY `get pods/ephemeralcontainers` cannot `get
# pods`, yet a GET on the ephemeralcontainers subresource returns the ENTIRE
# parent Pod object (metadata + spec + status), not an ephemeral-only
# projection -- the subresource store is a struct copy of the parent pod store.
# So "ephemeralcontainers read" is in fact full-Pod read. Observed by comparing
# the attacker's subresource read to the admin's `get pod`.
# Mechanism: ../transitions/get-status-reads-parent.md (and
#            ../transitions/pod-ephemeralcontainers-updates-pod.md)
#   p = get pods/ephemeralcontainers ; q = get pods ; when = same name+namespace.
source "$(dirname "$0")/../lib.sh"
test_start "pods-ephemeralcontainers-reads-parent"

ns="$(new_ns pod-ec-read)"
secret_marker="wp-marker-${RUN_ID}"

# Pre-existing state: a Pod carrying a distinctive field (an annotation) that
# only a *full-object* read would reveal -- proving the subresource GET returns
# the parent, not an ephemeralcontainers-only slice.
step "admin creates a Pod carrying a distinctive annotation"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create pod"
apiVersion: v1
kind: Pod
metadata:
  name: target
  annotations: { wp-e2e/marker: "${secret_marker}" }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" target 90

mk_sa "$ns" attacker
# ONLY the read on the ephemeralcontainers subresource. NOT `get pods`.
grant_ns "$ns" attacker "" "pods/ephemeralcontainers" "get"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" get pods/ephemeralcontainers "$ns"
assert_cannot "$subj" get pods                     "$ns"   # THE GAP

# ACT (as attacker, exercising ONLY p): GET the ephemeralcontainers subresource.
raw="/api/v1/namespaces/$ns/pods/target/ephemeralcontainers"
step "attacker GETs the ephemeralcontainers subresource of Pod/target"
body="$(as_subj "$subj" get --raw "$raw" 2>/dev/null)" \
  || fail "attacker could not read pods/ephemeralcontainers"
assert_nonempty "$body" "ephemeralcontainers subresource body"

# EFFECT: the returned body is the whole Pod -- it carries fields that are not
# part of ephemeralcontainers (kind=Pod, the spec container, and the private
# annotation). A read scoped to the subresource yielded the full parent object.
assert_contains "$body" "\"kind\":\"Pod\""        "subresource GET returns a full Pod object"
assert_contains "$body" "$secret_marker"          "parent-only annotation leaked via the subresource read"
assert_contains "$body" "\"image\":\"${IMG}\""    "parent spec (container image) leaked via the subresource read"
step "attacker read the full parent Pod (annotation '$secret_marker' and spec) through 'get pods/ephemeralcontainers' alone, without 'get pods'"

test_pass
