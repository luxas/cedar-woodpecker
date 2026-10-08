#!/usr/bin/env bash
# Implicit permission: update volumeattachments/status -> update volumeattachments
#                                        (volumeattachment-status-updates-parent)
#
# Proof: an identity granted ONLY `update/patch volumeattachments/status` cannot
# `update volumeattachments`, yet writing the `/status` subresource mutates the
# ONE stored (cluster-scoped) VolumeAttachment object. The status REST is a
# struct copy of the parent store with only the update strategy swapped. We flip
# `status.attached` via /status and read it back on the parent with a plain
# `get volumeattachments` (as admin). We create our OWN uniquely-named dummy VA
# with a FAKE attacher/nodeName/PV and never touch the real node's attachments.
# Mechanism: ../transitions/subresource-status-networking-storage.md
source "$(dirname "$0")/../lib.sh"
test_start "volumeattachment-status-updates-parent"

if ! K api-resources --api-group=storage.k8s.io 2>/dev/null | grep -qw volumeattachments; then
  echo "RESULT: SKIP volumeattachment-status-updates-parent storage.k8s.io VolumeAttachment API not present"
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

# A bootstrap namespace just to host the attacker SA (the VA itself is cluster-scoped).
home_ns="$(new_ns va-status-home)"; wp_track "namespace/$home_ns"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY the volumeattachments/status subresource cluster-wide (get/update/patch --
# a merge-patch of a subresource first GETs it, so `get` is still status-only
# access, i.e. p). Plus a parent read, purely to OBSERVE.
grant_cluster "$home_ns" attacker "storage.k8s.io" "volumeattachments/status" "get,update,patch"
grant_cluster "$home_ns" attacker "storage.k8s.io" "volumeattachments"        "get,list"

# Our OWN uniquely-named dummy VolumeAttachment -- fake attacher/node/PV, so no
# CSI controller reconciles it and we never touch the real node.
va="$(rid wp-va)"; va="$(tr '[:upper:]' '[:lower:]' <<<"$va")"
_track "volumeattachment/$va"; wp_track "volumeattachment/$va"
step "admin creates dummy VolumeAttachment '$va' (fake attacher/node/PV)"
K apply -f - >/dev/null <<EOF || fail "could not create dummy VolumeAttachment"
apiVersion: storage.k8s.io/v1
kind: VolumeAttachment
metadata: { name: $va }
spec:
  attacher: wp.example.com
  nodeName: wp-fake-node-${RUN_ID}
  source:
    persistentVolumeName: wp-fake-pv-${RUN_ID}
EOF

assert_can    "$subj" update volumeattachments/status   # holds p
assert_cannot "$subj" update volumeattachments          # THE GAP: lacks q
assert_cannot "$subj" patch  volumeattachments

step "attacker PATCHes volumeattachments/status to forge status.attached=true (never touching the parent verb)"
as_subj "$subj" patch volumeattachment "$va" --subresource=status --type=merge -p \
  '{"status":{"attached":true}}' >/dev/null || fail "attacker could not patch volumeattachments/status"

# EFFECT: the write landed on the parent VolumeAttachment -- `get
# volumeattachments` (the q the attacker lacks) returns the forged attach state.
got=""
for _ in $(seq 1 10); do
  got="$(K get volumeattachment "$va" -o jsonpath='{.status.attached}' 2>/dev/null)"
  [ "$got" = "true" ] && break
  sleep 1
done
assert_eq "$got" "true" "status write via /status reached the parent VolumeAttachment object"
step "attacker's /status write (attached=true) is visible on the parent VolumeAttachment via 'get volumeattachments'"

test_pass
