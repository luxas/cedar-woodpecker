#!/usr/bin/env bash
# Implicit permission: delete Namespace -> delete Pod
#                                                (namespace-delete-deletes-pods)
#
# Proof: an identity granted ONLY `delete namespaces` (NOT `delete pods`) can
# destroy a Pod it never had delete rights on: deleting the parent Namespace
# makes the namespace controller cascade-delete every namespaced object inside.
# Mechanism: ../transitions/namespace-delete-cascades.md
#   when: resource.namespace.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-delete-deletes-pods"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-del-pods-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY delete+get namespaces, cluster-wide (+read pods purely to observe the
# cascade). Deliberately NO `delete pods`.
grant_cluster "$home_ns" attacker "" "namespaces" "delete,get,list"
grant_cluster "$home_ns" attacker "" "pods" "get,list"

# Admin creates a throwaway target namespace with a running Pod inside.
target_ns="$(new_ns ns-del-pods-target)"
step "admin creates a running victim Pod in $target_ns"
K -n "$target_ns" apply -f - >/dev/null <<EOF || fail "could not create victim pod"
apiVersion: v1
kind: Pod
metadata: { name: victim }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$target_ns" victim 90

assert_can    "$subj" delete namespaces
assert_cannot "$subj" delete pods "$target_ns"

step "attacker deletes Namespace $target_ns (holding only 'delete namespaces')"
as_subj "$subj" delete namespace "$target_ns" --wait=false >/dev/null \
  || fail "attacker could not delete the namespace"

# EFFECT: the namespace controller cascade-deletes the Pod the attacker could
# not delete directly.
wait_gone "$target_ns" pod victim 90
step "namespace controller cascade-deleted the Pod on behalf of an identity holding only 'delete namespaces'"

test_pass
