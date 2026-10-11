#!/usr/bin/env bash
# Implicit permission: create pods/exec -> get Secret  (pod-exec-reads-secret)
#
# Proof: an identity granted `create pods/exec` (plus get pods to target one)
# but NOT `get secrets` can exec into a running Pod that mounts a Secret and
# `cat` the mounted file -- reading the Secret's bytes without holding `get
# secrets`. The kubelet has already mounted the Secret into the container the
# exec session enters (runtime data path, not an apiserver read).
# Mechanism: ../transitions/pod-exec-reads-secret.md
#   when: resource1.namespace == resource.namespace   (namespace-scoped)
source "$(dirname "$0")/../lib.sh"
test_start "pod-exec-reads-secret"

ns="$(new_ns pod-exec)"
secret_val="s3cr3t-${RUN_ID}"

# Pre-existing state (ADMIN authors the pod + secret): a running Pod that mounts
# a Secret the attacker must never be able to `get` directly.
step "admin creates Secret 'target' and a Pod 'vault' that mounts it"
K -n "$ns" create secret generic target --from-literal=token="$secret_val" >/dev/null \
  || fail "could not create target secret"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create vault pod"
apiVersion: v1
kind: Pod
metadata: { name: vault }
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
    secret: { secretName: target }
EOF
wait_pod_running "$ns" vault 90

# Attacker identity: `create pods/exec` + `get pods` (to name the target), but
# deliberately NOT `get secrets` and NOT `create pods` (it authors nothing).
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods/exec" "create"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p (create pods/exec) but not q (get secrets).
assert_can    "$subj" create pods/exec "$ns"
assert_cannot "$subj" get    secrets   "$ns"

# ACT (as attacker, exercising ONLY p): exec into the Pod and read the mounted
# Secret file. The exec session enters the container the kubelet already
# populated with the Secret bytes.
step "attacker execs into Pod 'vault' and cats the mounted Secret"
got="$(as_subj "$subj" -n "$ns" exec vault -- cat /mnt/secret/token 2>/dev/null)"

# EFFECT: the attacker holds the Secret bytes it was never allowed to `get`.
assert_eq "$got" "$secret_val" "attacker read the Secret via exec without 'get secrets'"
step "exec session read the mounted Secret bytes ($got) with only 'create pods/exec'"

test_pass
