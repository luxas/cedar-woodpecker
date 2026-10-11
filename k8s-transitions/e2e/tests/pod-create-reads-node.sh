#!/usr/bin/env bash
# Implicit permission: create Pod -> get Node  (pod-create-reads-node)
#
# Proof: an identity granted ONLY `create pods` cannot `get nodes`, yet by
# setting spec.nodeName it pins the Pod to a Node, and the control plane
# resolves that Node to place/run the Pod -- the Node authorizer pod->node edge
# (pod.Spec.NodeName). We make the Node resolution observable as an attacker:
# the SAME create, SAME RBAC, pinned to the REAL node reaches Running (a kubelet
# that read the pod on that node claimed it), while pinned to a FAKE node name
# it is admitted but never runs (no node resolves it) -- stays Pending with no
# node binding. The run-vs-stall outcome is decided by whether the named Node
# exists, i.e. by a Node resolution the attacker cannot perform itself.
# Mechanism: ../transitions/pod-create-reads-node.md
#   when: resource1.nodeName == resource.name   (Node is cluster-scoped)
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-node"

ns="$(new_ns pod-rd-node)"
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create pods) but not q (get nodes). Node is cluster-scoped.
assert_can    "$subj" create pods       "$ns"
assert_cannot "$subj" get    nodes               # cluster-scoped get

# Discover the real node AS ADMIN (the attacker cannot -- that is the point).
node="$(K get nodes -o jsonpath='{.items[0].metadata.name}')"
assert_nonempty "$node" "admin resolved a real node name"
fake="ghost-$(rid node)"; fake="$(tr '[:upper:]' '[:lower:]' <<<"$fake")"

node_pod() {  # node_pod POD_NAME NODE_NAME
cat <<EOF
apiVersion: v1
kind: Pod
metadata: { name: $1 }
spec:
  nodeName: $2
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
}

# ACT 1 (as attacker): create a Pod pinned to the REAL node.
step "attacker creates a Pod pinned to real node '$node'"
node_pod real "$node" | as_subj "$subj" -n "$ns" create -f - >/dev/null \
  || fail "attacker could not create the real-node pod"

# ACT 2 (as attacker): create a Pod pinned to a FAKE node.
step "attacker creates a Pod pinned to nonexistent node '$fake'"
node_pod fake "$fake" | as_subj "$subj" -n "$ns" create -f - >/dev/null \
  || fail "attacker could not create the fake-node pod"

# EFFECT (observed as admin): the real-node pod runs (its Node was resolved and
# a kubelet read it to run it); the fake-node pod is admitted but never binds to
# a running node. The contrast proves Node resolution gated placement.
wait_pod_running "$ns" real 90
hostip="$(K -n "$ns" get pod real -o jsonpath='{.status.hostIP}')"
assert_nonempty "$hostip" "real-node pod got a hostIP (bound to a real Node)"
bound_node="$(K -n "$ns" get pod real -o jsonpath='{.spec.nodeName}')"
assert_eq "$bound_node" "$node" "real-node pod is placed on the named Node"

# The fake-node pod must NOT be Running after a grace window (no node claims it).
sleep 8
fake_phase="$(K -n "$ns" get pod fake -o jsonpath='{.status.phase}')"
case "$fake_phase" in
  Running) fail "fake-node pod unexpectedly Running -- node resolution not load-bearing";;
  *) step "fake-node pod stuck in '$fake_phase' (no Node 'ghost' resolved to run it)";;
esac

step "Node resolution gated placement: pin to '$node' => Running on it (hostIP $hostip); pin to '$fake' => $fake_phase, both authored with only 'create pods'"

test_pass
