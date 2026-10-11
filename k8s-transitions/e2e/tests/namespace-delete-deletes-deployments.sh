#!/usr/bin/env bash
# Implicit permission: delete Namespace -> delete Deployment
#                                           (namespace-delete-deletes-deployments)
#
# Proof: an identity granted ONLY `delete namespaces` (NOT `delete deployments`)
# can destroy a Deployment it never had delete rights on: deleting the parent
# Namespace makes the namespace controller cascade-delete every namespaced
# object inside (apps/v1 Deployments included).
# Mechanism: ../transitions/namespace-delete-cascades.md
#   when: resource.namespace.name == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "namespace-delete-deletes-deployments"

# A bootstrap namespace to host the attacker SA.
home_ns="$(new_ns ns-del-dep-home)"
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
# ONLY delete+get namespaces, cluster-wide (+read deployments purely to observe
# the cascade). Deliberately NO `delete deployments`.
grant_cluster "$home_ns" attacker "" "namespaces" "delete,get,list"
grant_cluster "$home_ns" attacker "apps" "deployments" "get,list"

# Admin creates a throwaway target namespace with a Deployment inside.
target_ns="$(new_ns ns-del-dep-target)"
step "admin creates Deployment 'victim' in $target_ns"
K -n "$target_ns" apply -f - >/dev/null <<EOF || fail "could not create victim deployment"
apiVersion: apps/v1
kind: Deployment
metadata: { name: victim, labels: { app: victim } }
spec:
  replicas: 1
  selector: { matchLabels: { app: victim } }
  template:
    metadata: { labels: { app: victim } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF
wait_exists "$target_ns" deployment victim 30

assert_can    "$subj" delete namespaces
assert_cannot "$subj" delete deployments "$target_ns"

step "attacker deletes Namespace $target_ns (holding only 'delete namespaces')"
as_subj "$subj" delete namespace "$target_ns" --wait=false >/dev/null \
  || fail "attacker could not delete the namespace"

# EFFECT: the namespace controller cascade-deletes the Deployment the attacker
# could not delete directly.
wait_gone "$target_ns" deployment victim 90
step "namespace controller cascade-deleted the Deployment on behalf of an identity holding only 'delete namespaces'"

test_pass
