#!/usr/bin/env bash
# Implicit permission: create Pod -> get PersistentVolumeClaim  (pod-create-reads-pvc)
#
# Proof: an identity granted ONLY `create pods` cannot `get
# persistentvolumeclaims`, yet by authoring a Pod with a
# volumes[].persistentVolumeClaim.claimName the scheduler/kubelet reads the PVC
# (and the bound PV) to attach the volume on the identity's behalf -- the Node
# authorizer pod->pvc edge. We observe the effect operationally: the Pod with
# the referenced PVC reaches Running, and the admin sees the PVC bound to it.
# Mechanism: ../transitions/pod-create-reads-pvc.md
#   when: resource1.namespace == resource.namespace
#         && resource1.pvcRefs.contains(resource.name)
source "$(dirname "$0")/../lib.sh"
test_start "pod-create-reads-pvc"

ns="$(new_ns pod-pvc)"

# Pre-existing cluster state: admin pre-creates a PVC the attacker must never
# read directly. The default storageclass is WaitForFirstConsumer, so the PVC
# stays Pending until a Pod consumes it -- which is exactly the attacker's act.
step "admin pre-creates PVC 'data' (1Mi, default storageclass)"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create target pvc"
apiVersion: v1
kind: PersistentVolumeClaim
metadata: { name: data }
spec:
  accessModes: ["ReadWriteOnce"]
  resources: { requests: { storage: 1Mi } }
EOF

# Attacker identity: ONLY `create pods` (plus get pods, to see its own pod).
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: the attacker holds p (create pods) but not q (get pvc).
assert_can    "$subj" create pods                   "$ns"
assert_cannot "$subj" get    persistentvolumeclaims "$ns"

# ACT (as attacker, exercising ONLY p): author a Pod referencing the PVC.
step "attacker creates a Pod referencing PVC 'data'"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the pod"
apiVersion: v1
kind: Pod
metadata: { name: mounter }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
    volumeMounts: [{ name: v, mountPath: /mnt/data }]
  volumes:
  - name: v
    persistentVolumeClaim: { claimName: data }
EOF

# EFFECT: the control plane read the PVC on the attacker's behalf to attach the
# volume -- observable as (a) the PVC transitioning to Bound (the provisioner
# resolved and bound it only because a Pod referenced it) and (b) the Pod with
# the referenced volume reaching Running. Observed as admin (attacker lacks get
# pvc, so it cannot even see the Bound status it caused).
wait_pod_running "$ns" mounter 90
phase="$(K -n "$ns" get pvc data -o jsonpath='{.status.phase}')"
assert_eq "$phase" "Bound" "PVC bound after a Pod the attacker authored referenced it"
vol="$(K -n "$ns" get pvc data -o jsonpath='{.spec.volumeName}')"
assert_nonempty "$vol" "PVC resolved to a bound PV"
step "kubelet/attach path read PVC 'data' (now Bound to $vol) for a Pod authored with only 'create pods'"

test_pass
