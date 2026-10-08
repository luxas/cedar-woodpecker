#!/usr/bin/env bash
# Implicit permission: create ServiceAccount/token -> get ServiceAccount (be the SA)
#                                            (serviceaccount-token-is-sa-credential)
#
# Proof: an identity granted ONLY `create serviceaccounts/token` for an SA cannot
# `get serviceaccounts`, yet minting a token via the TokenRequest subresource
# yields a signed bearer credential that authenticates AS that SA -- strictly
# stronger than `get`. We mint the token (p) and prove the token IS the SA's
# identity: `kubectl --token=$T auth whoami` reports
# system:serviceaccount:<ns>:<sa>. Possessing the token is being the SA.
# Mechanism: ../transitions/serviceaccount-token-is-sa-credential.md
source "$(dirname "$0")/../lib.sh"
test_start "serviceaccount-token-is-sa-credential"

ns="$(new_ns sa-token)"

# The target SA whose identity will be minted. It needs no special rights; we
# only prove the minted token authenticates AS it.
mk_sa "$ns" target
target_subj="$(sa_subject "$ns" target)"

# Attacker identity: ONLY `create serviceaccounts/token` on the target SA. The
# TokenRequest subresource grant is the explicit permission p; it must NOT be
# able to `get serviceaccounts` (the modelled target q), nor to read tokens any
# other way.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "serviceaccounts/token" "create"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create serviceaccounts/token) but not q (get serviceaccounts).
assert_can    "$subj" create serviceaccounts/token "$ns"
assert_cannot "$subj" get    serviceaccounts        "$ns"

# ACT (as attacker, exercising ONLY p): mint a token for the target SA.
# `kubectl create token` POSTs the serviceaccounts/token subresource.
step "attacker mints a token for SA '$ns/target' via the TokenRequest subresource"
tok="$(as_subj "$subj" -n "$ns" create token target 2>/dev/null)"
assert_nonempty "$tok" "TokenRequest returned a token"
step "attacker obtained a signed bearer token for the SA (len=${#tok})"

# EFFECT: presenting the token authenticates AS the target SA. Observed with the
# token itself (no admin needed): the credential's power is intrinsic.
#
# We must authenticate with ONLY the token -- NOT the admin kubeconfig context,
# whose client certificate would otherwise win over --token and report
# kubernetes-admin. So point at the cluster's server directly with an EMPTY
# kubeconfig (KUBECONFIG=/dev/null) and only the bearer token.
export PATH="$HOME/.local/bin:$PATH"
cluster="$(K config view -o jsonpath='{.contexts[?(@.name=="'"$CTX"'")].context.cluster}')"
server="$(K config view -o jsonpath='{.clusters[?(@.name=="'"$cluster"'")].cluster.server}')"
assert_nonempty "$server" "resolved the API server URL for context $CTX"
whoami_tok() { KUBECONFIG=/dev/null kubectl --server="$server" --insecure-skip-tls-verify=true --token="$tok" "$@"; }
who="$(whoami_tok auth whoami -o jsonpath='{.status.userInfo.username}' 2>/dev/null)"
[ -z "$who" ] && who="$(whoami_tok auth whoami 2>/dev/null | grep -oE 'system:serviceaccount:[^ ]+' | head -1)"
assert_eq "$who" "$target_subj" "token authenticates as the target ServiceAccount"
step "the minted token authenticates AS '$who' -- possessing the token IS being the SA, from only 'create serviceaccounts/token'"

test_pass
