#!/usr/bin/env bash
# Implicit permission: patch Secret -> patch Pod  (secret-remounts-pod)
#
# Proof: an identity granted ONLY `patch secrets` (not `patch pods`, not `get
# pods`) can change what an already-running Pod mounts -- the kubelet's secret
# manager re-reads the referenced Secret and refreshes the mounted volume file
# in the live container, without any Pod write. So `patch secrets` effectively
# exercises `patch pods` (the effective config of the mounting Pod) on the
# identity's behalf.
# Mechanism: ../transitions/secret-remounts-pod.md
#   when: resource.namespace == resource1.namespace
#         && resource.secretRefs.contains(resource1.name)
source "$(dirname "$0")/../lib.sh"
test_start "secret-remounts-pod"

ns="$(new_ns sec-remount)"
old_val="old-${RUN_ID}"
new_val="PWNED-${RUN_ID}"

# Pre-existing state (ADMIN): a Secret and a running Pod that mounts it as a
# volume. Volume-mounted secrets are refreshed in place by the kubelet (unlike
# env/subPath, which are frozen at start) -- the right surface for this proof.
step "admin creates Secret 'mounted' (token=$old_val) and a Pod 'app' that mounts it"
K -n "$ns" create secret generic mounted --from-literal=token="$old_val" >/dev/null \
  || fail "could not create secret"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create app pod"
apiVersion: v1
kind: Pod
metadata: { name: app }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
    volumeMounts: [{ name: s, mountPath: /mnt/secret, readOnly: true }]
  volumes:
  - name: s
    secret: { secretName: mounted }
EOF
wait_pod_running "$ns" app 90

# Baseline: the running pod currently sees the OLD value.
base="$(K -n "$ns" exec app -- cat /mnt/secret/token 2>/dev/null)"
assert_eq "$base" "$old_val" "pod initially mounts the old secret value"

# Attacker identity: ONLY `patch secrets` (+ get secrets to target it).
# Deliberately NOT patch/update pods and NOT get/exec pods.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "secrets" "patch,get"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (patch secrets) but not q (patch pods).
assert_can    "$subj" patch secrets "$ns"
assert_cannot "$subj" patch pods    "$ns"
assert_cannot "$subj" update pods   "$ns"

# ACT (as attacker, exercising ONLY p): patch the Secret's bytes.
step "attacker patches Secret 'mounted' token -> $new_val (holds only 'patch secrets')"
new_b64="$(printf '%s' "$new_val" | base64 | tr -d '\n')"
as_subj "$subj" -n "$ns" patch secret mounted --type=merge \
  -p "{\"data\":{\"token\":\"$new_b64\"}}" >/dev/null \
  || fail "attacker could not patch the secret"

# EFFECT: the kubelet refreshes the mounted file in the RUNNING pod the attacker
# never wrote to. Poll up to ~90s (kubelet sync period). Observed as admin.
step "polling the running Pod for the kubelet to re-mount the new value (up to 120s)"
got=""; i=0
while [ $i -lt 120 ]; do
  got="$(K -n "$ns" exec app -- cat /mnt/secret/token 2>/dev/null)"
  [ "$got" = "$new_val" ] && break
  sleep 3; i=$((i+3))
done
assert_eq "$got" "$new_val" "kubelet refreshed the mounted Secret in the live Pod (after ${i}s)"
step "patch secrets changed what Pod 'app' runs with (token now '$got') without ever holding 'patch pods'"

test_pass
