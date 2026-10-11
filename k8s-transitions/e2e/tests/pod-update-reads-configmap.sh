#!/usr/bin/env bash
# Implicit permission: update/patch Pod -> get ConfigMap  (pod-update-reads-configmap)
#
# HONEST SKIP. The symbolic transition is sound (if a Pod's configMapRefs change,
# the Node authorizer rebuilds the pod->configmap edge and the kubelet mounts the
# newly-referenced ConfigMap). But the OPERATIONAL precondition -- an attacker
# holding `update pods` MUTATING an existing Pod to add a configMap reference --
# is rejected by the apiserver: `ValidatePodUpdate` forbids changing any field
# other than spec.containers[*].image / initContainers[*].image /
# activeDeadlineSeconds / tolerations / terminationGracePeriodSeconds. None of
# those pulls in a ConfigMap, so `update pods` cannot add a configMap ref on a
# vanilla cluster. We prove the wall (the Forbidden) rather than fake a pass.
# Mechanism: ../transitions/pod-update-reads-configmap.md
source "$(dirname "$0")/../lib.sh"
test_start "pod-update-reads-configmap"

ns="$(new_ns pod-upd-cm)"

# Pre-existing state: a ConfigMap the attacker must never read, and a running
# Pod (admin-authored) that does NOT yet reference it.
K -n "$ns" create configmap target --from-literal=key="v-${RUN_ID}" >/dev/null \
  || fail "could not create target configmap"
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

# Attacker identity: ONLY `update,patch pods` (+ get pods). NOT get configmaps.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "update,patch,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP is real: holds p (update pods), lacks q (get configmaps).
assert_can    "$subj" update pods       "$ns"
assert_can    "$subj" patch  pods       "$ns"
assert_cannot "$subj" get    configmaps "$ns"

# ATTEMPT p: mutate the running Pod to add a configMap volume+mount. The
# apiserver rejects it (pod spec immutability), so the kubelet never reads the
# ConfigMap -- the operational edge cannot be produced.
step "attacker attempts to add a configMap volume via patch pods (expected: Forbidden)"
err="$(as_subj "$subj" -n "$ns" patch pod base --type=json -p \
  '[{"op":"add","path":"/spec/volumes","value":[{"name":"cm","configMap":{"name":"target"}}]},{"op":"add","path":"/spec/containers/0/volumeMounts","value":[{"name":"cm","mountPath":"/mnt/cm"}]}]' 2>&1)"
rc=$?
[ $rc -eq 0 ] && fail "pod update to add a configMap ref UNEXPECTEDLY succeeded -- revisit this SKIP"
assert_contains "$err" "Forbidden" "apiserver forbids the mutation that would add the ConfigMap ref"
step "confirmed wall: $(echo "$err" | grep -o 'Forbidden: pod updates may not change[^\"]*' | head -1)"

printf 'RESULT: SKIP %s %s\n' "$_TESTNAME" \
  "update pods cannot add a configMap ref -- ValidatePodUpdate forbids all but image/activeDeadlineSeconds/tolerations/terminationGracePeriodSeconds; no operational edge on a vanilla cluster"
exit 3
