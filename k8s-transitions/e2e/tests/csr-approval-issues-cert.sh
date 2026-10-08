#!/usr/bin/env bash
# Implicit permission: update CSR/approval -> update CSR / issue a client cert
#                                                       (csr-approval-issues-cert)
#
# Transition claim: holding ONLY `update certificatesigningrequests/approval`
# lets you approve a CSR, after which the signer issues a client cert
# (status.certificate) -- a minted identity -- without `sign`.
#
# OPERATIONAL FINDING (why this SKIPs): on a real apiserver (incl. this kind
# v1.37 cluster) the `update .../approval` grant ALONE is NOT sufficient to flip
# the Approved condition for a built-in signerName. The apiserver's CSR approval
# admission additionally authorizes an `approve` verb on the `signers` resource
# scoped to the CSR's signerName. With only `update .../approval`, the approval
# write is rejected:
#     Forbidden: user not permitted to approve requests with signerName
#     "kubernetes.io/kube-apiserver-client"
# So the modelled source permission `p` (update .../approval) cannot by itself
# perform the modelled effect `q` on a live cluster -- the operational `p` is
# "update .../approval AND approve signers/<signerName>". The signer controller
# IS active here (verified out-of-band: granting the signer `approve` too makes
# the exact same approval succeed and status.certificate populate), so the SKIP
# is not "signer absent" -- it is "the single-permission transition is not
# operationally reproducible; approval is gated on the signer verb as well".
#
# This test proves that gate exists (grant ONLY p, show the approval is Forbidden)
# and then SKIPs with the exact reason, per the contract's honest-SKIP rule. It
# writes nothing durable to shared cluster state beyond uniquely-named, tracked
# objects that the harness cleans up.
# Mechanism: ../transitions/csr-approval-issues-cert.md
source "$(dirname "$0")/../lib.sh"
test_start "csr-approval-issues-cert"
export PATH="$HOME/.local/bin:$PATH"

need() { command -v "$1" >/dev/null 2>&1 || { printf 'RESULT: SKIP %s %s not on PATH (needed to build a CSR)\n' "$_TESTNAME" "$1"; exit 3; }; }
need openssl

csr_name="$(rid csr-approve)"
_track "certificatesigningrequest/$csr_name"
cn="wp-e2e-user-${RUN_ID}-$RANDOM"

# Build a real client-auth CSR (PKCS#10) for an arbitrary CN -- the "new identity".
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' RETURN
openssl req -new -newkey rsa:2048 -nodes \
  -keyout "$tmp/key.pem" -out "$tmp/req.pem" -subj "/CN=${cn}/O=wp-e2e" >/dev/null 2>&1 \
  || fail "could not generate a CSR with openssl"
req_b64="$(base64 -w0 < "$tmp/req.pem" 2>/dev/null || base64 < "$tmp/req.pem" | tr -d '\n')"

# Admin creates the pending CSR with the built-in client signer and client-auth
# usages. (Creating a CSR is self-service; we do it as admin so the attacker's
# ONLY power under test is the approval subresource write.)
step "admin creates a pending CSR '$csr_name' (signer kubernetes.io/kube-apiserver-client)"
K apply -f - >/dev/null <<EOF || fail "could not create the CSR"
apiVersion: certificates.k8s.io/v1
kind: CertificateSigningRequest
metadata: { name: $csr_name }
spec:
  request: $req_b64
  signerName: kubernetes.io/kube-apiserver-client
  usages: ["client auth"]
EOF

# Attacker identity: ONLY `update certificatesigningrequests/approval` (+ get on
# approval, + get/list/watch on the parent to observe). CSRs are cluster-scoped.
# It explicitly does NOT hold `approve` on signers, nor `sign`.
home_ns="$(new_ns csr-home)"
mk_sa "$home_ns" attacker
grant_cluster "$home_ns" attacker "certificates.k8s.io" "certificatesigningrequests/approval" "update,get"
grant_cluster "$home_ns" attacker "certificates.k8s.io" "certificatesigningrequests" "get,list,watch"
subj="$(sa_subject "$home_ns" attacker)"

# THE GAP: holds p (update .../approval) but not q (update the parent CSR) and
# not the signer `approve` escape hatch.
assert_can    "$subj" update certificatesigningrequests/approval
assert_cannot "$subj" update certificatesigningrequests
step "attacker holds update CSR/approval but NOT update on the parent CSR"

# ACT (as attacker, exercising ONLY p): attempt to approve via the /approval
# subresource. On a real apiserver this is REJECTED unless the subject also holds
# `approve` on signers/<signerName> -- which is exactly the finding.
step "attacker attempts to approve the CSR via the /approval subresource"
approve_err="$(as_subj "$subj" certificate approve "$csr_name" 2>&1)"; approve_rc=$?

if [ $approve_rc -eq 0 ]; then
  # Unexpected on a stock apiserver: approval succeeded with only the subresource
  # write. If so, the transition IS operationally reproducible here; verify the
  # signer issues the cert and PASS.
  cond="$(K get csr "$csr_name" -o jsonpath='{.status.conditions[?(@.type=="Approved")].type}' 2>/dev/null)"
  assert_eq "$cond" "Approved" "CSR carries the Approved condition after the attacker's /approval write"
  issued=""
  for i in $(seq 1 60); do
    issued="$(K get csr "$csr_name" -o jsonpath='{.status.certificate}' 2>/dev/null)"
    [ -n "$issued" ] && break; sleep 1
  done
  if [ -z "$issued" ]; then
    printf 'RESULT: SKIP %s approval succeeded with only update .../approval, but the built-in signer did not populate status.certificate within 60s on this cluster\n' "$_TESTNAME"
    exit 3
  fi
  pem="$(printf '%s' "$issued" | base64 -d 2>/dev/null)"
  assert_contains "$pem" "BEGIN CERTIFICATE" "status.certificate is a PEM client certificate"
  subj_line="$(printf '%s' "$pem" | openssl x509 -noout -subject 2>/dev/null)"
  assert_contains "$subj_line" "$cn" "the issued certificate's subject carries the requested CN"
  step "signer issued a client cert for CN=$cn via an approval made with only 'update certificatesigningrequests/approval'"
  test_pass
fi

# Expected path: approval was Forbidden on the signerName gate. Confirm it is the
# signerName authorization (not some unrelated error), then SKIP honestly.
case "$approve_err" in
  *"not permitted to approve requests with signerName"*)
    printf 'RESULT: SKIP %s update certificatesigningrequests/approval ALONE cannot approve a CSR: the apiserver also authorizes "approve" on signers/<signerName>, so approval was Forbidden ("not permitted to approve requests with signerName kubernetes.io/kube-apiserver-client"). The signer controller IS active (granting approve signers/<signerName> makes the identical approval succeed and status.certificate populate); the single-permission transition is therefore not operationally reproducible -- operational p is "update .../approval AND approve signers/<signerName>".\n' "$_TESTNAME"
    exit 3
    ;;
  *)
    fail "approval failed for an unexpected reason (rc=$approve_rc): $approve_err"
    ;;
esac
