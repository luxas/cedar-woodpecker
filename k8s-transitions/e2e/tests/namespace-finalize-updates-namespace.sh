#!/usr/bin/env bash
# Implicit permission: update namespaces/finalize -> update Namespace
#                                           (namespace-finalize-updates-namespace)
#
# Proof: an identity granted ONLY `update namespaces/finalize` (NOT `update
# namespaces`) can mutate the parent Namespace object. The /finalize subresource
# shares the Namespace's storage, so a PUT to /finalize writes the parent's
# spec.finalizers. The attacker changes spec.finalizers on a live (Active)
# Namespace it has no `update namespaces` right to touch; admin observes the
# parent object's spec.finalizers changed on its behalf.
# Mechanism: ../transitions/namespace-finalize.md
#   when: resource.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-finalize-updates-namespace"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-finup-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY update namespaces/finalize, cluster-wide (+get to observe). Deliberately
# NO `update namespaces` and NO `delete namespaces`.
grant_cluster "$home_ns" attacker "" "namespaces/finalize" "update"
grant_cluster "$home_ns" attacker "" "namespaces" "get,list"

# Admin creates a live (Active) target namespace. It is NOT deleted, so clearing
# or setting finalizers only mutates the parent -- it must not vanish.
target_ns="$(new_ns ns-finup-target)"
marker="example.com/added-via-finalize"
step "admin creates live Namespace $target_ns (finalizers: [kubernetes])"
before="$(K get namespace "$target_ns" -o jsonpath='{.spec.finalizers}' 2>/dev/null)"
assert_eq "$before" '["kubernetes"]' "baseline parent finalizers before the attack"

assert_can    "$subj" update namespaces/finalize
assert_cannot "$subj" update namespaces

step "attacker PUTs /finalize adding '$marker' (holding only 'update namespaces/finalize')"
as_subj "$subj" replace --raw "/api/v1/namespaces/$target_ns/finalize" -f - >/dev/null <<EOF \
  || fail "attacker could not write the finalize subresource"
{"apiVersion":"v1","kind":"Namespace","metadata":{"name":"$target_ns"},"spec":{"finalizers":["kubernetes","$marker"]}}
EOF

# EFFECT: the parent Namespace object -- which the attacker cannot `update`
# directly -- now carries the attacker's finalizer. The namespace is still
# Active (a pure update, not a delete).
after="$(K get namespace "$target_ns" -o jsonpath='{.spec.finalizers}' 2>/dev/null)"
assert_contains "$after" "$marker" "parent Namespace spec.finalizers mutated via /finalize"
phase="$(K get namespace "$target_ns" -o jsonpath='{.status.phase}' 2>/dev/null)"
assert_eq "$phase" "Active" "namespace was updated, not deleted"
step "parent Namespace was mutated (finalizers now: $after) by an identity holding only 'update namespaces/finalize'"

# Restore finalizers to [kubernetes] (as admin) so harness cleanup can actually
# delete the namespace -- a lingering custom finalizer would park it in
# Terminating forever on the shared cluster.
K replace --raw "/api/v1/namespaces/$target_ns/finalize" -f - >/dev/null 2>&1 <<EOF || true
{"apiVersion":"v1","kind":"Namespace","metadata":{"name":"$target_ns"},"spec":{"finalizers":["kubernetes"]}}
EOF

test_pass
