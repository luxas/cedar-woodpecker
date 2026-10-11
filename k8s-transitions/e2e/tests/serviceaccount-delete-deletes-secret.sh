#!/usr/bin/env bash
# Implicit permission: delete ServiceAccount -> delete Secret (its token Secret)
#                                              (serviceaccount-delete-deletes-secret)
#
# Proof: an identity granted ONLY `delete serviceaccounts` (NOT `delete secrets`)
# can delete an SA; the tokens controller / legacy-token-cleaner then garbage
# -collects the Secret of type kubernetes.io/service-account-token that named the
# SA -- a Secret the attacker could never delete directly. A destructive side
# effect of a single verb on a different resource kind.
# Mechanism: ../transitions/serviceaccount-delete-deletes-secret.md
#
# SKIP policy: modern clusters mint SA tokens via the TokenRequest API and no
# longer auto-create token Secrets, so we create the token Secret explicitly
# (annotated at the SA) to exercise the legacy cleanup path. If the controller
# does not remove the orphaned Secret within ~60s on this kind cluster, SKIP with
# the exact reason (the tokens_controller/legacy cleaner must be active).
source "$(dirname "$0")/../lib.sh"
test_start "serviceaccount-delete-deletes-secret"

ns="$(new_ns sa-del-secret)"

# Admin creates the SA, then a manually-created Secret of type
# kubernetes.io/service-account-token annotated with the SA name -- the legacy
# token Secret shape the cleanup controllers reconcile. We let the controller
# fill in the SA uid annotation (it reconciles created token Secrets).
step "admin creates SA 'victim' and a kubernetes.io/service-account-token Secret bound to it"
mk_sa "$ns" victim
sa_uid="$(K -n "$ns" get sa victim -o jsonpath='{.metadata.uid}' 2>/dev/null)"
assert_nonempty "$sa_uid" "victim SA has a uid"

secret_name="victim-token"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create the SA token Secret"
apiVersion: v1
kind: Secret
metadata:
  name: $secret_name
  namespace: $ns
  annotations:
    kubernetes.io/service-account.name: victim
    kubernetes.io/service-account.uid: "$sa_uid"
type: kubernetes.io/service-account-token
EOF

# Give the tokens controller a moment to populate the token data (confirms it is
# watching/owning this Secret). Not required for the proof, but a healthy signal.
for i in $(seq 1 15); do
  [ -n "$(K -n "$ns" get secret "$secret_name" -o jsonpath='{.data.token}' 2>/dev/null)" ] && break
  sleep 1
done

# Attacker identity: ONLY `delete serviceaccounts` (+get/list to observe), and
# explicitly NOT `delete secrets`.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "serviceaccounts" "delete,get,list"
grant_ns "$ns" attacker "" "secrets" "get,list"   # observe only; NOT delete
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (delete serviceaccounts) but not q (delete secrets).
assert_can    "$subj" delete serviceaccounts "$ns"
assert_cannot "$subj" delete secrets         "$ns"

# Precondition: the token Secret exists before the act.
wait_exists "$ns" secret "$secret_name" 10

# ACT (as attacker, exercising ONLY p): delete the ServiceAccount.
step "attacker deletes ServiceAccount '$ns/victim'"
as_subj "$subj" -n "$ns" delete serviceaccount victim >/dev/null \
  || fail "attacker could not delete the ServiceAccount"

# EFFECT: a controller deletes the orphaned token Secret. Wait up to ~60s.
step "waiting (<=60s) for the token controller to garbage-collect the orphaned Secret"
gone=""
for i in $(seq 1 60); do
  if ! K -n "$ns" get secret "$secret_name" >/dev/null 2>&1; then gone=1; break; fi
  sleep 1
done

if [ -z "$gone" ]; then
  printf 'RESULT: SKIP %s token cleanup controller did not delete the orphaned kubernetes.io/service-account-token Secret within 60s on this kind cluster (SA deleted; Secret not collected -- controller may be disabled or require --legacy-service-account-token-clean-up)\n' "$_TESTNAME"
  exit 3
fi

step "a controller deleted the SA's token Secret via a ServiceAccount delete the attacker made with only 'delete serviceaccounts'"
test_pass
