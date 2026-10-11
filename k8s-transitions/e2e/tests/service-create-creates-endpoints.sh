#!/usr/bin/env bash
# Implicit permission: create Service -> create Endpoints  (service-create-creates-endpoints)
#
# Proof: an identity granted ONLY `create services` cannot `create endpoints`,
# yet after it creates a Service *with a selector* the endpoints controller
# (kube-controller-manager) creates an Endpoints object named after the Service
# on the identity's behalf. Explicit p = "create a Service"; implicit q =
# "create the Endpoints object the controller derives from the selector".
# Spec: ../transitions/service-create-creates-endpoints.json
#   when: resource.namespace == context.resource1.namespace
source "$(dirname "$0")/../lib.sh"
test_start "service-create-creates-endpoints"

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

ns="$(new_ns svc-ep)"; _LOCAL_CLEAN+=("namespace/$ns")
mk_sa "$ns" attacker
# ONLY services (+read endpoints to OBSERVE). Deliberately NO endpoints write.
grant_ns "$ns" attacker "" "services"  "create,get,list"
grant_ns "$ns" attacker "" "endpoints" "get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create services) but not q (create endpoints).
assert_can    "$subj" create services  "$ns"
assert_cannot "$subj" create endpoints "$ns"

# ACT (as attacker, exercising ONLY p): create a Service WITH A SELECTOR. The
# selector is what makes the endpoints controller manage an Endpoints object for
# it (a selector-less/headless-manual Service would not be controller-managed).
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

# EFFECT: the endpoints controller creates an Endpoints object named after the
# Service -- an object the attacker could not create directly. Observed as admin
# (attacker has read-only endpoints just to be sure, but the proof is the object
# existing by the controller's hand).
wait_exists "$ns" endpoints "$svc" 60
owner_svc="$(K -n "$ns" get endpoints "$svc" -o jsonpath='{.metadata.name}')"
assert_eq "$owner_svc" "$svc" "Endpoints object is named after the Service"
# The endpoints controller stamps this label on the objects it manages.
mgr="$(K -n "$ns" get endpoints "$svc" -o jsonpath='{.metadata.labels.endpoints\.kubernetes\.io/managed-by}')"
assert_eq "$mgr" "endpoint-controller" "Endpoints is managed by the endpoints controller, not the attacker"
step "endpoints controller created Endpoints/$svc (managed-by=$mgr) for a Service the attacker made with only 'create services'"

test_pass
