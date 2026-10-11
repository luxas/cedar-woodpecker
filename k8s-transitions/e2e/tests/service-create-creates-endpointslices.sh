#!/usr/bin/env bash
# Implicit permission: create Service -> create EndpointSlice
#                                              (service-create-creates-endpointslices)
#
# Proof: an identity granted ONLY `create services` cannot `create
# endpointslices` (discovery.k8s.io), yet after it creates a Service *with a
# selector* the endpointslice controller (kube-controller-manager) creates an
# EndpointSlice labeled kubernetes.io/service-name=<svc> on the identity's
# behalf. Explicit p = "create a Service"; implicit q = "create the
# EndpointSlice the controller derives from the selector".
# Spec: ../transitions/service-create-creates-endpointslices.json
#   when: resource.namespace == context.resource1.namespace
source "$(dirname "$0")/../lib.sh"
test_start "service-create-creates-endpointslices"

# Local cleanup is defensive redundancy: lib.sh uses a FILE-backed cleanup
# ledger (WP_CLEANUP_FILE) that survives the $(...) subshell of new_ns /
# grant_cluster, so the harness _cleanup already deletes tracked objects on
# exit. We additionally track our own objects here (and, for cluster-scoped
# tests, create them with names we control) and chain our trap ahead of
# lib's _cleanup.
_LOCAL_CLEAN=()
_lclean() { local rc=$?; for ((i=${#_LOCAL_CLEAN[@]}-1; i>=0; i--)); do
  K delete "${_LOCAL_CLEAN[$i]}" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done; return $rc; }
trap '_lclean; _cleanup' EXIT

ns="$(new_ns svc-eps)"; _LOCAL_CLEAN+=("namespace/$ns")
mk_sa "$ns" attacker
# ONLY services (+read endpointslices to OBSERVE). Deliberately NO
# endpointslices write.
grant_ns "$ns" attacker "" "services" "create,get,list"
grant_ns "$ns" attacker "discovery.k8s.io" "endpointslices" "get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create services) but not q (create endpointslices).
assert_can    "$subj" create services       "$ns"
assert_cannot "$subj" create endpointslices "$ns"

# ACT (as attacker, exercising ONLY p): create a Service WITH A SELECTOR -- the
# selector is what makes the endpointslice controller manage slices for it.
svc="shopsvc"
step "attacker creates Service '$svc' with a selector"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the service"
apiVersion: v1
kind: Service
metadata: { name: ${svc} }
spec:
  selector: { app: shop }
  ports: [{ port: 80, targetPort: 8080 }]
EOF

# EFFECT: the endpointslice controller creates an EndpointSlice labeled
# kubernetes.io/service-name=<svc> -- an object the attacker could not create
# directly. Observed as admin.
wait_exists "$ns" endpointslices "kubernetes.io/service-name=$svc" 60
slice="$(K -n "$ns" get endpointslices -l "kubernetes.io/service-name=$svc" -o jsonpath='{.items[0].metadata.name}')"
assert_nonempty "$slice" "an EndpointSlice for the Service exists"
# The endpointslice controller stamps this managed-by label on slices it owns.
mgr="$(K -n "$ns" get endpointslices "$slice" -o jsonpath='{.metadata.labels.endpointslice\.kubernetes\.io/managed-by}')"
assert_eq "$mgr" "endpointslice-controller.k8s.io" "EndpointSlice is managed by the endpointslice controller, not the attacker"
step "endpointslice controller created EndpointSlice/$slice (service-name=$svc, managed-by=$mgr) for a Service the attacker made with only 'create services'"

test_pass
