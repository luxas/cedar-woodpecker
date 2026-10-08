#!/usr/bin/env bash
# Implicit permission: create Namespace -> create ServiceAccount
#                                                 (namespace-create-seeds-serviceaccount)
#
# Proof: an identity granted ONLY `create namespaces` cannot `create
# serviceaccounts`, yet after it creates a Namespace the control plane seeds a
# `default` ServiceAccount (and its token) into it on the identity's behalf.
# Mechanism: ../transitions/namespace-create-seeds-defaults.md
source "$(dirname "$0")/../lib.sh"
test_start "namespace-create-seeds-serviceaccount"

# A bootstrap namespace just to host the attacker SA.
home_ns="$(new_ns ns-seed-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY namespaces, cluster-wide (+read to observe the seeded SA).
grant_cluster "$home_ns" attacker "" "namespaces" "create,get"
grant_cluster "$home_ns" attacker "" "serviceaccounts" "get,list"

target_ns="$(rid ns-seed-target)"; target_ns="$(tr '[:upper:]' '[:lower:]' <<<"$target_ns")"
_track "namespace/$target_ns"

assert_can    "$subj" create namespaces
assert_cannot "$subj" create serviceaccounts "$target_ns"

step "attacker creates Namespace $target_ns"
as_subj "$subj" create namespace "$target_ns" >/dev/null || fail "attacker could not create the namespace"

# EFFECT: the ServiceAccount controller seeds the 'default' SA the attacker
# could not create directly.
wait_exists "$target_ns" serviceaccount default 60
step "control plane seeded the 'default' ServiceAccount into a Namespace the attacker made with only 'create namespaces'"

test_pass
