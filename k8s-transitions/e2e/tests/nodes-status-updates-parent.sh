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

# Local cleanup is defensive redundancy: lib.sh uses a FILE-backed cleanup
# ledger (WP_CLEANUP_FILE) that survives the $(...) subshell of new_ns /
# grant_cluster, so the harness _cleanup already deletes tracked objects on
# exit. We additionally track our own objects here (and, for cluster-scoped
# tests, create them with names we control) and chain our trap ahead of
# lib's _cleanup.
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
step "attacker APPENDS a WPProbe condition via nodes/status (never touching the parent verb)"
# Use a JSON Patch `add` to APPEND to the conditions list, rather than a merge
# patch that would REPLACE the whole list and wipe the kubelet's real
# conditions (Ready/MemoryPressure/...), transiently flipping the Node to
# Unknown. Appending leaves the real conditions intact and the Node Ready.
as_subj "$subj" patch node "$node" --subresource=status --type=json -p \
  "[{\"op\":\"add\",\"path\":\"/status/conditions/-\",\"value\":{\"type\":\"WPProbe\",\"status\":\"True\",\"reason\":\"wp\",\"message\":\"$marker\",\"lastHeartbeatTime\":\"2024-01-01T00:00:00Z\",\"lastTransitionTime\":\"2024-01-01T00:00:00Z\"}}]" \
  >/dev/null || fail "attacker could not patch nodes/status"

# EFFECT: the write landed on the parent Node -- `get nodes` (the q the attacker
# lacks) returns, AMONG the conditions, the one the attacker wrote via /status.
got=""
for _ in $(seq 1 10); do
  got="$(K get node "$node" -o jsonpath='{range .status.conditions[?(@.type=="WPProbe")]}{.message}{"\n"}{end}' 2>/dev/null)"
  printf '%s\n' "$got" | grep -qxF "$marker" && break
  sleep 1
done
printf '%s\n' "$got" | grep -qxF "$marker" \
  || fail "status write via /status did not reach the parent Node (WPProbe messages seen: $(echo $got))"
# The real conditions must still be present (append must not have wiped them).
assert_contains "$(K get node "$node" -o jsonpath='{.status.conditions[*].type}')" "Ready" \
  "the kubelet's real Node conditions were left intact (append, not replace)"
step "attacker's /status write ($marker) is visible on the parent Node via 'get nodes'; Ready preserved"

# Remove EVERY forged WPProbe condition (admin), one at a time by current index,
# leaving the kubelet's real conditions untouched -- keeps the shared Node
# pristine (also mops up any WPProbe left by an earlier run).
for _ in $(seq 1 10); do
  idx="$(K get node "$node" -o json 2>/dev/null | python3 -c \
    "import json,sys; cs=json.load(sys.stdin)['status']['conditions']; print(next((i for i,c in enumerate(cs) if c.get('type')=='WPProbe'),-1))" 2>/dev/null)"
  [ "${idx:--1}" -ge 0 ] || break
  K patch node "$node" --subresource=status --type=json -p \
    "[{\"op\":\"test\",\"path\":\"/status/conditions/$idx/type\",\"value\":\"WPProbe\"},{\"op\":\"remove\",\"path\":\"/status/conditions/$idx\"}]" \
    >/dev/null 2>&1 || break
done

test_pass
