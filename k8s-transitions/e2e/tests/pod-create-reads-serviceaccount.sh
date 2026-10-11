#!/usr/bin/env bash
# Implicit permission: create Pod -> get ServiceAccount  (pod-create-reads-serviceaccount)
#
# Proof: an identity granted ONLY `create pods` cannot `get serviceaccounts`,
# yet by setting spec.serviceAccountName=X on a Pod it creates, the
# kubelet/apiserver read that ServiceAccount to project its token into the Pod
# -- the Node authorizer pod->serviceaccount edge. We observe the effect: the
# Pod runs with that SA's projected token, and the admin confirms the SA was
# read (the token the kubelet minted is bound to SA X).
# Mechanism: ../transitions/pod-create-reads-serviceaccount.md
#   when: resource1.namespace == resource.namespace
#         && resource1.serviceAccountRef == resource.name
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-serviceaccount"

ns="$(new_ns pod-sa)"

# Pre-existing state: a target ServiceAccount the attacker must never read.
step "admin creates target ServiceAccount 'runas'"
mk_sa "$ns" runas

# Attacker identity: ONLY `create pods` (plus get pods, to see its own pod).
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p (create pods) but not q (get serviceaccounts).
assert_can    "$subj" create pods            "$ns"
assert_cannot "$subj" get    serviceaccounts "$ns"

# ACT (as attacker, exercising ONLY p): author a Pod that runs as SA 'runas'
# with a projected serviceAccountToken, forcing the SA to be read.
step "attacker creates a Pod with serviceAccountName=runas + a projected token"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the pod"
apiVersion: v1
kind: Pod
metadata: { name: runner }
spec:
  serviceAccountName: runas
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
    volumeMounts: [{ name: tok, mountPath: /var/run/secrets/tok, readOnly: true }]
  volumes:
  - name: tok
    projected:
      sources:
      - serviceAccountToken: { path: token, expirationSeconds: 3600 }
EOF

# EFFECT: the apiserver/kubelet read SA 'runas' to mint a token bound to it, and
# the Pod runs as that SA. Observed as admin (attacker lacks get secrets/sa).
wait_pod_running "$ns" runner 90
runas_sa="$(K -n "$ns" get pod runner -o jsonpath='{.spec.serviceAccountName}')"
assert_eq "$runas_sa" "runas" "pod admitted running as the referenced SA"
# The minted token the kubelet projected is a JWT whose subject is SA 'runas':
# decode its payload and confirm the SA name. (Admin reads it via exec; the
# read of SA 'runas' is what the kubelet did on the attacker's behalf.)
tok="$(K -n "$ns" exec runner -- cat /var/run/secrets/tok/token 2>/dev/null)"
assert_nonempty "$tok" "projected token present (kubelet minted it for SA runas)"
payload="$(echo "$tok" | cut -d. -f2)"
# base64url -> base64 pad, decode
pad=$(( (4 - ${#payload} % 4) % 4 )); payload="${payload}$(printf '=%.0s' $(seq 1 $pad))"
claims="$(echo "$payload" | tr '_-' '/+' | base64 -d 2>/dev/null)"
assert_contains "$claims" "serviceaccount" "token minted for a serviceaccount"
assert_contains "$claims" "runas" "token subject is SA 'runas' -- the SA the kubelet read"
step "kubelet read SA 'runas' and minted its bound token for a Pod authored with only 'create pods'"

test_pass
