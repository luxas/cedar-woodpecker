#!/usr/bin/env bash
# Implicit permission: create Pod -> get Namespace  (pod-create-reads-namespace)
#
# Proof: an identity granted ONLY `create pods` (cluster-wide) cannot `get
# namespaces`, yet every namespaced create drives the apiserver's
# NamespaceExists admission plugin to READ the target Namespace object (and
# reject the create if it is absent/terminating). We make that read observable
# as an attacker: the SAME create, with the SAME RBAC, SUCCEEDS into an existing
# namespace but is REJECTED with "namespaces \"X\" not found" into a missing
# one. The pass/fail is decided by a Namespace read the attacker cannot perform
# itself -- so admission read the Namespace on the attacker's behalf.
# Mechanism: ../transitions/pod-create-reads-namespace.md
#   when: resource1.namespace.name == resource.name
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-namespace"

# A bootstrap namespace to host the attacker SA; it is the EXISTING target.
ns="$(new_ns pod-rd-ns)"
mk_sa "$ns" attacker
subj="$(sa_subject "$ns" attacker)"
# ONLY create pods, cluster-wide (so the same grant covers any namespace).
# Deliberately NOT get namespaces.
grant_cluster "$ns" attacker "" "pods" "create,get,list"

# THE GAP: holds p (create pods) but not q (get namespaces).
assert_can    "$subj" create pods       "$ns"
assert_cannot "$subj" get    namespaces          # cluster-scoped get

# A namespace name that does NOT exist.
missing="no-such-$(rid ns)"; missing="$(tr '[:upper:]' '[:lower:]' <<<"$missing")"

pod_yaml() {  # pod_yaml NAME
cat <<EOF
apiVersion: v1
kind: Pod
metadata: { name: $1 }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
}

# ACT 1 (as attacker): create a Pod into the EXISTING namespace -> admission's
# namespace read passes, create succeeds.
step "attacker creates a Pod into existing namespace '$ns' (expect success)"
out_ok="$(pod_yaml ok | as_subj "$subj" -n "$ns" create -f - 2>&1)"; rc_ok=$?
[ $rc_ok -eq 0 ] || fail "create into existing namespace failed: $out_ok"
step "created: $out_ok"

# ACT 2 (as attacker): the SAME create into a MISSING namespace -> admission's
# namespace read fails, create is rejected BECAUSE the namespace does not exist
# (not an RBAC denial -- the attacker holds create pods cluster-wide).
step "attacker creates the same Pod into missing namespace '$missing' (expect namespace NotFound)"
out_miss="$(pod_yaml nope | as_subj "$subj" -n "$missing" create -f - 2>&1)"; rc_miss=$?
[ $rc_miss -eq 0 ] && fail "create into a missing namespace UNEXPECTEDLY succeeded"
# Must be a namespace-not-found (admission read), NOT a forbidden/RBAC error.
assert_contains "$out_miss" "not found" "rejection is a namespace read miss, not RBAC"
assert_contains "$out_miss" "namespace" "rejection names the missing Namespace object"
case "$out_miss" in *orbidden*) fail "rejection was RBAC (forbidden), not a namespace read: $out_miss";; esac

# EFFECT: the create outcome is gated by a Namespace read the attacker could not
# do (assert_cannot get namespaces above). Admission read the Namespace for it.
step "admission read the Namespace: create gated on it ('$ns' ok, '$missing' => $(echo "$out_miss" | tr -d '\n'))"

test_pass
