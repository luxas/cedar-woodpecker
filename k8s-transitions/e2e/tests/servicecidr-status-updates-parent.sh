#!/usr/bin/env bash
# Implicit permission: update servicecidrs/status -> update servicecidrs
#                                            (servicecidr-status-updates-parent)
#
# Proof: an identity granted ONLY `update/patch servicecidrs/status` cannot
# `update servicecidrs`, yet writing the `/status` subresource mutates the ONE
# stored (cluster-scoped) ServiceCIDR object -- the status store reuses the
# parent REST store with servicecidr.StatusStrategy. We write a bogus `WPProbe`
# status condition via /status and read it back on the parent with a plain
# `get servicecidrs` (as admin). We create our OWN uniquely-named ServiceCIDR
# with a small, non-overlapping range (TEST-NET-3 203.0.113.0/29) and clean it
# up; we never touch the cluster's real ServiceCIDR.
# Mechanism: ../transitions/subresource-status-networking-storage.md
source "$(dirname "$0")/../lib.sh"
test_start "servicecidr-status-updates-parent"

if ! K api-resources --api-group=networking.k8s.io 2>/dev/null | grep -qw servicecidrs; then
  echo "RESULT: SKIP servicecidr-status-updates-parent networking.k8s.io ServiceCIDR API not present on this cluster"
  exit 3
fi

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

# Bootstrap namespace to host the attacker SA (ServiceCIDR is cluster-scoped).
home_ns="$(new_ns sc-status-home)"; wp_track "namespace/$home_ns"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
grant_cluster "$home_ns" attacker "networking.k8s.io" "servicecidrs/status" "get,update,patch"
grant_cluster "$home_ns" attacker "networking.k8s.io" "servicecidrs"        "get,list"

# Our OWN uniquely-named ServiceCIDR with a small non-overlapping range.
sc="$(rid wp-cidr)"; sc="$(tr '[:upper:]' '[:lower:]' <<<"$sc")"
_track "servicecidr/$sc"; wp_track "servicecidr/$sc"
step "admin creates ServiceCIDR '$sc' (non-overlapping 203.0.113.0/29)"
if ! K apply -f - >/dev/null 2>/tmp/wp-sc-$$.err <<EOF
apiVersion: networking.k8s.io/v1
kind: ServiceCIDR
metadata: { name: $sc }
spec:
  cidrs: ["203.0.113.0/29"]
EOF
then
  reason="$(tr '\n' ' ' </tmp/wp-sc-$$.err | sed 's/  */ /g')"; rm -f /tmp/wp-sc-$$.err
  echo "RESULT: SKIP servicecidr-status-updates-parent ServiceCIDR creation refused: ${reason:-unknown}"
  exit 3
fi
rm -f /tmp/wp-sc-$$.err

assert_can    "$subj" update servicecidrs/status   # holds p
assert_cannot "$subj" update servicecidrs          # THE GAP: lacks q
assert_cannot "$subj" patch  servicecidrs

marker="wp-sc-${RUN_ID}"
step "attacker PATCHes servicecidrs/status with a WPProbe condition (never touching the parent verb)"
as_subj "$subj" patch servicecidr "$sc" --subresource=status --type=merge -p \
  "{\"status\":{\"conditions\":[{\"type\":\"WPProbe\",\"status\":\"True\",\"reason\":\"wp\",\"message\":\"$marker\",\"lastTransitionTime\":\"2024-01-01T00:00:00Z\"}]}}" \
  >/dev/null || fail "attacker could not patch servicecidrs/status"

# EFFECT: the write landed on the parent ServiceCIDR -- `get servicecidrs`
# (the q the attacker lacks) returns the condition the attacker wrote via /status.
got=""
for _ in $(seq 1 10); do
  got="$(K get servicecidr "$sc" -o jsonpath='{.status.conditions[?(@.type=="WPProbe")].message}' 2>/dev/null)"
  [ "$got" = "$marker" ] && break
  sleep 1
done
assert_eq "$got" "$marker" "status write via /status reached the parent ServiceCIDR object"
step "attacker's /status write ($marker) is visible on the parent ServiceCIDR via 'get servicecidrs'"

test_pass
