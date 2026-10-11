#!/usr/bin/env bash
# Implicit permission: delete Namespace -> delete Secret
#                                                (namespace-delete-deletes-secrets)
#
# Proof: an identity granted ONLY `delete namespaces` (NOT `delete secrets`) can
# destroy a Secret it never had delete rights on: deleting the parent Namespace
# makes the namespace controller cascade-delete every namespaced object inside.
# Mechanism: ../transitions/namespace-delete-cascades.md
#   when: resource.namespace.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-delete-deletes-secrets"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-del-sec-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY delete+get namespaces, cluster-wide (+read secrets purely to observe the
# cascade). Deliberately NO `delete secrets`.
grant_cluster "$home_ns" attacker "" "namespaces" "delete,get,list"
grant_cluster "$home_ns" attacker "" "secrets" "get,list"

# Admin creates a throwaway target namespace with a Secret inside.
target_ns="$(new_ns ns-del-sec-target)"
step "admin creates Secret 'victim' in $target_ns"
K -n "$target_ns" create secret generic victim --from-literal=token=s3cr3t >/dev/null \
  || fail "could not create victim secret"
wait_exists "$target_ns" secret victim 30

assert_can    "$subj" delete namespaces
assert_cannot "$subj" delete secrets "$target_ns"

step "attacker deletes Namespace $target_ns (holding only 'delete namespaces')"
as_subj "$subj" delete namespace "$target_ns" --wait=false >/dev/null \
  || fail "attacker could not delete the namespace"

# EFFECT: the namespace controller cascade-deletes the Secret the attacker could
# not delete directly.
wait_gone "$target_ns" secret victim 90
step "namespace controller cascade-deleted the Secret on behalf of an identity holding only 'delete namespaces'"

test_pass
