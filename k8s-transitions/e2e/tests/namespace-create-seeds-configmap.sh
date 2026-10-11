#!/usr/bin/env bash
# Implicit permission: create Namespace -> create ConfigMap
#                                                (namespace-create-seeds-configmap)
#
# Proof: an identity granted ONLY `create namespaces` cannot `create
# configmaps`, yet after it creates a Namespace the root-ca-cert-publisher seeds
# the `kube-root-ca.crt` ConfigMap into it on the identity's behalf.
# Mechanism: ../transitions/namespace-create-seeds-defaults.md
#   when: resource.namespace.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-create-seeds-configmap"

# A bootstrap namespace just to host the attacker SA.
home_ns="$(new_ns ns-cm-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY namespaces, cluster-wide (+read to observe the seeded ConfigMap).
grant_cluster "$home_ns" attacker "" "namespaces" "create,get"
grant_cluster "$home_ns" attacker "" "configmaps" "get,list"

target_ns="$(rid ns-cm-target)"; target_ns="$(tr '[:upper:]' '[:lower:]' <<<"$target_ns")"
_track "namespace/$target_ns"

assert_can    "$subj" create namespaces
assert_cannot "$subj" create configmaps "$target_ns"

step "attacker creates Namespace $target_ns"
as_subj "$subj" create namespace "$target_ns" >/dev/null || fail "attacker could not create the namespace"

# EFFECT: the root-ca-cert-publisher seeds the kube-root-ca.crt ConfigMap the
# attacker could not create directly.
wait_exists "$target_ns" configmap kube-root-ca.crt 60
# And it genuinely carries the API server CA bundle.
got="$(K -n "$target_ns" get configmap kube-root-ca.crt -o jsonpath='{.data.ca\.crt}' 2>/dev/null)"
assert_contains "$got" "BEGIN CERTIFICATE" "seeded ConfigMap carries the CA bundle"
step "control plane seeded the 'kube-root-ca.crt' ConfigMap into a Namespace the attacker made with only 'create namespaces'"

test_pass
