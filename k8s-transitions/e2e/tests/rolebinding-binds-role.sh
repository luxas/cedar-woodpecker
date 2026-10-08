#!/usr/bin/env bash
# Implicit permission: create RoleBinding -> bind Role   (rolebinding-binds-role)
#
# Proof: an identity granted `create rolebindings` (and holding some rules R, so
# the escalation check passes) cannot `bind` roles explicitly, yet creating a
# RoleBinding that references a Role grants that Role's rules to a subject -- the
# `bind` verb's effect -- without ever holding `bind`.
# Mechanism: ../transitions/rolebinding-binds-role.md
source "$(dirname "$0")/../lib.sh"
test_start "rolebinding-binds-role"

ns="$(new_ns rb-bind)"

# A Role the attacker will hand out. Its single rule (get configmaps) is also
# granted to the attacker, so RBAC's ConfirmNoEscalation permits the binding
# WITHOUT the attacker holding the dedicated `bind` verb.
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create reader role"
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: { name: reader, namespace: $ns }
rules:
- apiGroups: [""]
  resources: ["configmaps"]
  verbs: ["get","list"]
EOF

mk_sa "$ns" attacker
mk_sa "$ns" victim
grant_ns "$ns" attacker "rbac.authorization.k8s.io" "rolebindings" "create,get,list"
grant_ns "$ns" attacker "" "configmaps" "get,list"   # the rules it is allowed to pass on
attacker="$(sa_subject "$ns" attacker)"
victim="$(sa_subject "$ns" victim)"

assert_can    "$attacker" create rolebindings "$ns"
assert_cannot "$attacker" bind   roles.rbac.authorization.k8s.io "$ns"
# Precondition: the victim starts with none of the Role's rules.
assert_cannot "$victim"   get    configmaps "$ns"

step "attacker creates a RoleBinding granting Role/reader to the victim SA"
as_subj "$attacker" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the rolebinding"
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata: { name: pwn, namespace: $ns }
roleRef: { apiGroup: rbac.authorization.k8s.io, kind: Role, name: reader }
subjects:
- { kind: ServiceAccount, name: victim, namespace: $ns }
EOF

# EFFECT: the victim now holds the Role's rules -- the attacker exercised `bind`
# through `create rolebindings` without ever holding the `bind` verb.
assert_can "$victim" get configmaps "$ns"
step "victim gained 'get configmaps' via a binding the attacker made with only 'create rolebindings'"

test_pass
