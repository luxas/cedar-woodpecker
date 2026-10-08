#!/usr/bin/env bash
# Implicit permission: update/patch Pod -> get Secret  (pod-update-reads-secret)
#
# HONEST SKIP. The symbolic transition is sound (if a Pod's secretRefs change,
# the Node authorizer rebuilds the pod->secret edge and the kubelet mounts the
# newly-referenced Secret). But the OPERATIONAL precondition -- an attacker
# holding `update pods` MUTATING an existing Pod to add a Secret reference -- is
# rejected by the apiserver: `ValidatePodUpdate` forbids changing any field
# other than spec.containers[*].image / initContainers[*].image /
# activeDeadlineSeconds / tolerations / terminationGracePeriodSeconds. None of
# those pulls in a Secret, so `update pods` cannot add a Secret ref on a vanilla
# cluster. (The Secret-read-via-Pod attack is the CREATE path, proven live in
# pod-create-reads-secret.sh.) We prove the wall rather than fake a pass.
# Mechanism: ../transitions/pod-update-reads-secret.md
source "$(dirname "$0")/../lib.sh"
test_start "pod-update-reads-secret"

ns="$(new_ns pod-upd-sec)"

# Pre-existing state: a Secret the attacker must never read, and a running Pod
# (admin-authored) that does NOT yet reference it.
K -n "$ns" create secret generic target --from-literal=token="s3cr3t-${RUN_ID}" >/dev/null \
  || fail "could not create target secret"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create base pod"
apiVersion: v1
kind: Pod
metadata: { name: base }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" base 90

# Attacker identity: ONLY `update,patch pods` (+ get pods). NOT get secrets.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "update,patch,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP is real: holds p (update pods), lacks q (get secrets).
assert_can    "$subj" update pods    "$ns"
assert_can    "$subj" patch  pods    "$ns"
assert_cannot "$subj" get    secrets "$ns"

# ATTEMPT p: mutate the running Pod to add a Secret volume+mount. The apiserver
# rejects it (pod spec immutability), so the kubelet never reads the Secret --
# the operational edge cannot be produced.
step "attacker attempts to add a Secret volume via patch pods (expected: Forbidden)"
err="$(as_subj "$subj" -n "$ns" patch pod base --type=json -p \
  '[{"op":"add","path":"/spec/volumes","value":[{"name":"s","secret":{"secretName":"target"}}]},{"op":"add","path":"/spec/containers/0/volumeMounts","value":[{"name":"s","mountPath":"/mnt/secret"}]}]' 2>&1)"
rc=$?
[ $rc -eq 0 ] && fail "pod update to add a Secret ref UNEXPECTEDLY succeeded -- revisit this SKIP"
assert_contains "$err" "Forbidden" "apiserver forbids the mutation that would add the Secret ref"
step "confirmed wall: $(echo "$err" | grep -o 'Forbidden: pod updates may not change[^\"]*' | head -1)"

printf 'RESULT: SKIP %s %s\n' "$_TESTNAME" \
  "update pods cannot add a Secret ref -- ValidatePodUpdate forbids all but image/activeDeadlineSeconds/tolerations/terminationGracePeriodSeconds; no operational edge on a vanilla cluster (the create path is proven in pod-create-reads-secret)"
exit 3
