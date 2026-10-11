#!/usr/bin/env bash
# Implicit permission: create ClusterRoleBinding -> bind ClusterRole (cluster scope)
#                                                  (clusterrolebinding-grants-all)
#
# Proof: an identity granted `create clusterrolebindings` (and holding some rules
# R, so the escalation check passes) cannot `bind` clusterroles explicitly, yet
# creating a ClusterRoleBinding that references a ClusterRole grants that
# ClusterRole's rules to a subject cluster-wide -- the `bind` verb's effect --
# without ever holding `bind`. This is the cluster-scoped sibling of
# rolebinding-binds-role; the real transition's `cluster-admin` reach is the same
# mechanism, demonstrated here on a scoped ClusterRole to keep the escalation
# check satisfiable without the `escalate`/`bind` escape hatch.
# Mechanism: ../transitions/clusterrolebinding-grants-all.md
source "$(dirname "$0")/../lib.sh"
test_start "clusterrolebinding-grants-all"

# A uniquely-named ClusterRole the attacker will hand out. Its single rule
# (get nodes -- a cluster-scoped read) is also granted to the attacker, so RBAC's
# ConfirmNoEscalation permits the binding WITHOUT the attacker holding the
# dedicated `bind` verb. Cluster-scoped object => unique name + tracked cleanup.
reader_role="$(rid cr-reader)"
_track "clusterrole/$reader_role"
K apply -f - >/dev/null <<EOF || fail "could not create reader clusterrole"
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata: { name: $reader_role }
rules:
- apiGroups: [""]
  resources: ["nodes"]
  verbs: ["get","list"]
EOF

# Home namespace just to host the two ServiceAccounts.
home_ns="$(new_ns crb-bind)"
mk_sa "$home_ns" attacker
mk_sa "$home_ns" victim

# Attacker: ONLY `create clusterrolebindings` (+get/list to observe), PLUS the
# exact rules it is allowed to pass on (get/list nodes), cluster-wide. Holding
# those rules is what lets ConfirmNoEscalation pass without `bind`.
grant_cluster "$home_ns" attacker "rbac.authorization.k8s.io" "clusterrolebindings" "create,get,list"
grant_cluster "$home_ns" attacker "" "nodes" "get,list"

attacker="$(sa_subject "$home_ns" attacker)"
victim="$(sa_subject "$home_ns" victim)"

assert_can    "$attacker" create clusterrolebindings
assert_cannot "$attacker" bind   clusterroles.rbac.authorization.k8s.io
# Precondition: the victim starts WITHOUT the ClusterRole's rules (cluster-wide).
assert_cannot "$victim"   get    nodes

# ACT (as attacker, exercising ONLY p): create a ClusterRoleBinding binding the
# ClusterRole to the victim SA. Unique binding name; track for cleanup.
crb_name="$(rid pwn-crb)"
_track "clusterrolebinding/$crb_name"
step "attacker creates a ClusterRoleBinding granting ClusterRole/$reader_role to the victim SA"
as_subj "$attacker" apply -f - >/dev/null <<EOF || fail "attacker could not create the clusterrolebinding"
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata: { name: $crb_name }
roleRef: { apiGroup: rbac.authorization.k8s.io, kind: ClusterRole, name: $reader_role }
subjects:
- { kind: ServiceAccount, name: victim, namespace: $home_ns }
EOF

# EFFECT: the victim now holds the ClusterRole's rules cluster-wide -- the
# attacker exercised `bind` through `create clusterrolebindings` without ever
# holding the `bind` verb.
assert_can "$victim" get nodes
step "victim gained cluster-wide 'get nodes' via a ClusterRoleBinding the attacker made with only 'create clusterrolebindings'"

test_pass
