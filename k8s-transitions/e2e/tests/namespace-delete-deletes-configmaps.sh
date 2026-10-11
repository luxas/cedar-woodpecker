#!/usr/bin/env bash
# Implicit permission: delete Namespace -> delete ConfigMap
#                                             (namespace-delete-deletes-configmaps)
#
# Proof: an identity granted ONLY `delete namespaces` (NOT `delete configmaps`)
# can destroy a ConfigMap it never had delete rights on: deleting the parent
# Namespace makes the namespace controller cascade-delete every namespaced
# object inside.
# Mechanism: ../transitions/namespace-delete-cascades.md
#   when: resource.namespace.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-delete-deletes-configmaps"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-del-cm-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY delete+get namespaces, cluster-wide (+read configmaps purely to observe
# the cascade). Deliberately NO `delete configmaps`.
grant_cluster "$home_ns" attacker "" "namespaces" "delete,get,list"
grant_cluster "$home_ns" attacker "" "configmaps" "get,list"

# Admin creates a throwaway target namespace with a uniquely-named ConfigMap
# inside (NOT the auto-seeded kube-root-ca.crt, so the assertion is unambiguous).
target_ns="$(new_ns ns-del-cm-target)"
step "admin creates ConfigMap 'victim' in $target_ns"
K -n "$target_ns" create configmap victim --from-literal=k=v >/dev/null \
  || fail "could not create victim configmap"
wait_exists "$target_ns" configmap victim 30

assert_can    "$subj" delete namespaces
assert_cannot "$subj" delete configmaps "$target_ns"

step "attacker deletes Namespace $target_ns (holding only 'delete namespaces')"
as_subj "$subj" delete namespace "$target_ns" --wait=false >/dev/null \
  || fail "attacker could not delete the namespace"

# EFFECT: the namespace controller cascade-deletes the ConfigMap the attacker
# could not delete directly.
wait_gone "$target_ns" configmap victim 90
step "namespace controller cascade-deleted the ConfigMap on behalf of an identity holding only 'delete namespaces'"

test_pass
