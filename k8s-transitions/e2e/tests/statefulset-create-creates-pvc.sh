#!/usr/bin/env bash
# Implicit permission: create StatefulSet -> create PersistentVolumeClaim
#   (statefulset-create-creates-pvc)
#
# Proof: an identity granted ONLY `create statefulsets` cannot `create
# persistentvolumeclaims`, yet after it creates a StatefulSet with
# volumeClaimTemplates the statefulset controller creates the per-replica PVC
# (named "<vct>-<sts>-<ordinal>") on its behalf. On this cluster the default
# StorageClass is WaitForFirstConsumer, so the PVC is CREATED (Pending) even
# before it binds -- and "create persistentvolumeclaims" is exactly q.
# Spec: ../transitions/statefulset-create-creates-pvc.json
#   p = create apps::statefulsets ; q = create core::persistentvolumeclaims ; same ns.
source "$(dirname "$0")/../lib.sh"
test_start "statefulset-create-creates-pvc"

ns="$(new_ns sts-pvc)"
mk_sa "$ns" attacker
# ONLY statefulsets (+read pvc to observe). Deliberately NO persistentvolumeclaims write.
grant_ns "$ns" attacker "apps" "statefulsets" "create,get,list"
grant_ns "$ns" attacker "" "persistentvolumeclaims" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create statefulsets            "$ns"
assert_cannot "$subj" create persistentvolumeclaims  "$ns"

step "attacker creates a StatefulSet with volumeClaimTemplates (replicas=1)"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the statefulset"
apiVersion: apps/v1
kind: StatefulSet
metadata: { name: stamp, labels: { app: stamp } }
spec:
  serviceName: stamp
  replicas: 1
  selector: { matchLabels: { app: stamp } }
  template:
    metadata: { labels: { app: stamp } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
        volumeMounts:
        - name: data
          mountPath: /data
  volumeClaimTemplates:
  - metadata: { name: data }
    spec:
      accessModes: ["ReadWriteOnce"]
      resources: { requests: { storage: 16Mi } }
EOF

# EFFECT: the per-replica PVC "data-stamp-0" the attacker could not create
# directly appears, created by the statefulset controller on its behalf.
# (WaitForFirstConsumer => it may be Pending; q is "create", so existence is the proof.)
wait_exists "$ns" pvc "data-stamp-0" 60
phase="$(K -n "$ns" get pvc data-stamp-0 -o jsonpath='{.status.phase}' 2>/dev/null)"
assert_nonempty "$phase" "PVC data-stamp-0 has a status phase (exists)"
step "statefulset controller created PVC 'data-stamp-0' (phase=$phase) from a StatefulSet the attacker made with only 'create statefulsets' -- attacker lacked 'create persistentvolumeclaims'"

test_pass
