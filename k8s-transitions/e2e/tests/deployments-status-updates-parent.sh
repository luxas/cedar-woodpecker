#!/usr/bin/env bash
# Implicit permission: update deployments/status -> update deployments
#                                               (deployments-status-updates-parent)
#
# Proof: an identity granted ONLY `update/patch deployments/status` cannot
# `update deployments`, yet writing the `/status` subresource mutates the ONE
# stored Deployment object -- the status endpoint is a struct copy of the parent
# store with only the update strategy swapped, so a status write IS a write to
# the parent. We write a bogus `WPProbe` status condition via /status and then
# read it back on the parent object with a plain `get deployments` (as admin).
# Mechanism: ../transitions/status-subresource-updates-parent.md
source "$(dirname "$0")/../lib.sh"
test_start "deployments-status-updates-parent"

# Self-cleanup: lib.sh's new_ns runs _track inside a command-substitution
# subshell, so under bash 4.2 + `set -u` the parent's _CLEANUP never sees the
# namespace and the EXIT trap deletes nothing. We record our own objects by name
# and delete them on exit, then chain the harness _cleanup. (Local helper only;
# lib.sh is untouched.)
WP_OBJS=()
wp_track() { WP_OBJS+=("$1"); }
wp_cleanup() {
  local rc=$? o
  for o in "${WP_OBJS[@]:-}"; do [ -n "$o" ] && K delete "$o" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done
  _cleanup || true
  return $rc
}
trap wp_cleanup EXIT

ns="$(new_ns dep-status)"; wp_track "namespace/$ns"

# Pre-existing object the attacker must not be able to update directly.
step "admin creates a Deployment the attacker may not update"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create deployment"
apiVersion: apps/v1
kind: Deployment
metadata: { name: victim, labels: { app: victim } }
spec:
  replicas: 1
  selector: { matchLabels: { app: victim } }
  template:
    metadata: { labels: { app: victim } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF

mk_sa "$ns" attacker
# ONLY the deployments/status subresource (get/update/patch -- a merge-patch of a
# subresource first GETs that same subresource, so `get` here is still status-only
# access, i.e. p). Plus a read of the parent, purely to OBSERVE our proof.
grant_ns "$ns" attacker "apps" "deployments/status" "get,update,patch"
grant_ns "$ns" attacker "apps" "deployments"        "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" update deployments/status "$ns"   # holds p
assert_cannot "$subj" update deployments        "$ns"   # THE GAP: lacks q
assert_cannot "$subj" patch  deployments        "$ns"

marker="wp-dep-${RUN_ID}"
step "attacker PATCHes deployments/status with a WPProbe condition (never touching the parent verb)"
as_subj "$subj" -n "$ns" patch deployment victim --subresource=status --type=merge -p \
  "{\"status\":{\"conditions\":[{\"type\":\"WPProbe\",\"status\":\"True\",\"reason\":\"wp\",\"message\":\"$marker\",\"lastUpdateTime\":\"2024-01-01T00:00:00Z\",\"lastTransitionTime\":\"2024-01-01T00:00:00Z\"}]}}" \
  >/dev/null || fail "attacker could not patch deployments/status"

# EFFECT: the write landed on the parent Deployment object -- a plain `get
# deployments` (the q the attacker lacks) returns the condition the attacker
# wrote through /status. Observed as admin.
got=""
for _ in $(seq 1 10); do
  got="$(K -n "$ns" get deployment victim -o jsonpath='{.status.conditions[?(@.type=="WPProbe")].message}' 2>/dev/null)"
  [ "$got" = "$marker" ] && break
  sleep 1
done
assert_eq "$got" "$marker" "status write via /status reached the parent Deployment object"
step "attacker's /status write ($marker) is visible on the parent Deployment via 'get deployments'"

test_pass
