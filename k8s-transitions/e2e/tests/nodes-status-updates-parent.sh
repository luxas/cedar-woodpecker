#!/usr/bin/env bash
# Implicit permission: update nodes/status -> update nodes
#                                              (nodes-status-updates-parent)
# Severity: CRITICAL -- a Node's status drives scheduling, eviction, and the
# node-lease/taint machinery.
#
# Proof: an identity granted ONLY `update/patch nodes/status` cannot `update
# nodes`, yet writing the `/status` subresource mutates the ONE stored
# (cluster-scoped) Node object. We forge a status condition / capacity on the
# node via /status and read it back on the parent with a plain `get nodes`.
# Mechanism: ../transitions/status-subresource-updates-parent.md
#
# !!! NODE-TOUCHING: mutates the single shared node and would disturb scheduling
# and other authors' pods. DO NOT RUN on the shared cluster. The parent runs it
# in isolation. The guard below exits 3 before any mutation; the real body is
# reachable only when run alone (WP_E2E_ALLOW_NODE=1).
source "$(dirname "$0")/../lib.sh"
if [ "${WP_E2E_ALLOW_NODE:-}" != "1" ]; then
  echo "RESULT: SKIP nodes-status-updates-parent mutates shared node; parent runs it isolated"
  exit 3
fi

test_start "nodes-status-updates-parent"

# Self-cleanup (see note in deployments-status-updates-parent.sh): new_ns's
# _track is lost to its subshell under bash 4.2 + set -u, so self-track and
# delete on exit, chaining the harness _cleanup.
WP_OBJS=()
wp_track() { WP_OBJS+=("$1"); }
wp_cleanup() {
  local rc=$? o
  for o in "${WP_OBJS[@]:-}"; do [ -n "$o" ] && K delete "$o" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done
  _cleanup || true
  return $rc
}
trap wp_cleanup EXIT

# The single kind node. We touch ONLY its /status, and only its own value, but
# this is still a shared-node mutation -- hence the isolation guard above.
node="$(K get nodes -o jsonpath='{.items[0].metadata.name}')"
assert_nonempty "$node" "found the node to target"

home_ns="$(new_ns node-status-home)"; wp_track "namespace/$home_ns"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY the nodes/status subresource cluster-wide (get/update/patch -- a
# merge-patch of a subresource first GETs it, so `get` is still status-only
# access, i.e. p). Plus a parent read, purely to OBSERVE.
grant_cluster "$home_ns" attacker "" "nodes/status" "get,update,patch"
grant_cluster "$home_ns" attacker "" "nodes"        "get,list"

assert_can    "$subj" update nodes/status   # holds p
assert_cannot "$subj" update nodes          # THE GAP: lacks q
assert_cannot "$subj" patch  nodes

marker="wp-node-${RUN_ID}"
step "attacker PATCHes nodes/status with a WPProbe condition (never touching the parent verb)"
as_subj "$subj" patch node "$node" --subresource=status --type=merge -p \
  "{\"status\":{\"conditions\":[{\"type\":\"WPProbe\",\"status\":\"True\",\"reason\":\"wp\",\"message\":\"$marker\",\"lastHeartbeatTime\":\"2024-01-01T00:00:00Z\",\"lastTransitionTime\":\"2024-01-01T00:00:00Z\"}]}}" \
  >/dev/null || fail "attacker could not patch nodes/status"

# EFFECT: the write landed on the parent Node -- `get nodes` (the q the attacker
# lacks) returns the condition the attacker wrote via /status.
got=""
for _ in $(seq 1 10); do
  got="$(K get node "$node" -o jsonpath='{.status.conditions[?(@.type=="WPProbe")].message}' 2>/dev/null)"
  [ "$got" = "$marker" ] && break
  sleep 1
done
assert_eq "$got" "$marker" "status write via /status reached the parent Node object"
step "attacker's /status write ($marker) is visible on the parent Node via 'get nodes'"

# NOTE: the forged WPProbe condition is left on the Node's status. There is no
# clean kubectl removal of a single status condition by type (a strategic merge
# cannot delete a list element here), and this test runs ONLY in the parent's
# isolated environment, which is torn down afterward. The kubelet continues to
# re-report the real Node conditions regardless; WPProbe is inert/cosmetic.

test_pass
