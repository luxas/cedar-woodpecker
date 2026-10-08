#!/usr/bin/env bash
# Implicit permission: update pods/status -> update Pod  (pods-status-updates-parent)
#
# Proof: an identity granted ONLY `update/patch pods/status` cannot `update
# pods`, yet writing the status subresource mutates the SAME stored Pod object
# (its .status) on its behalf -- StatusREST.Update calls straight through to the
# shared parent store. A write scoped to "status only" is a write to the Pod.
# Mechanism: ../transitions/status-subresource-updates-parent.md
#   p = update pods/status ; q = update pods ; when = same name+namespace.
source "$(dirname "$0")/../lib.sh"
test_start "pods-status-updates-parent"

ns="$(new_ns pod-st-upd)"

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
# ONLY the status subresource (update to perform p; patch is the same mechanism;
# get to confirm the write landed). Neither grants the parent `update pods`.
grant_ns "$ns" attacker "" "pods/status" "get,update,patch"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" update pods/status "$ns"
assert_cannot "$subj" update pods        "$ns"   # THE GAP

# ACT (as attacker, exercising ONLY p): patch the status subresource to stamp a
# distinctive condition onto the parent Pod's .status.
marker="wp-e2e-marker-${RUN_ID}"
step "attacker PATCHes the status subresource to set a custom Pod condition"
as_subj "$subj" -n "$ns" patch pod victim --subresource=status --type=merge -p \
  "{\"status\":{\"conditions\":[{\"type\":\"${marker}\",\"status\":\"True\",\"lastTransitionTime\":\"2026-10-08T00:00:00Z\",\"reason\":\"WpE2E\",\"message\":\"set-via-status-subresource\"}]}}" \
  >/dev/null || fail "attacker could not patch pods/status"

# EFFECT: the parent Pod's .status now carries the attacker-set condition -- a
# write to the Pod object the attacker could not perform via `update pods`.
# Observed as admin.
got="$(K -n "$ns" get pod victim -o jsonpath="{range .status.conditions[?(@.type==\"${marker}\")]}{.message}{end}")"
assert_eq "$got" "set-via-status-subresource" "parent Pod .status mutated via the status subresource"
step "apiserver wrote the parent Pod's .status (condition '$marker') via a subresource patch the attacker made with only 'update/patch pods/status'"

test_pass
