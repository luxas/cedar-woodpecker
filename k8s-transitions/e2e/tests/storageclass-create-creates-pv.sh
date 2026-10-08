#!/usr/bin/env bash
# Implicit permission (claimed): create StorageClass -> create PersistentVolume
#                                            (storageclass-create-creates-pv)
#
# HONEST SKIP. The literal source operation `p` is "create a StorageClass, and
# nothing else". A StorageClass is only a provisioning *template*; creating one
# ALONE never causes a PersistentVolume to be created. Dynamic provisioning is
# demand-driven and provisioner-mediated:
#
#   1. A PersistentVolumeClaim must reference the StorageClass by name
#      (spec.storageClassName == <sc>), AND
#   2. the StorageClass's external/in-tree provisioner must act on that PVC to
#      provision a PV, AND
#   3. with VolumeBindingMode=WaitForFirstConsumer (the mode of this cluster's
#      only/default StorageClass, rancher.io/local-path), provisioning is
#      deferred until a *Pod* consuming the PVC is scheduled -- so even a PVC is
#      not enough; a Pod is required too.
#
# So the real chain is: create StorageClass -> (someone) create PVC referencing
# it -> (WaitForFirstConsumer) a Pod consumes the PVC -> the provisioner creates
# a PV. The PV is created by the provisioner in response to a *consumed PVC*, not
# by the StorageClass's creator and not by the act of creating the StorageClass.
# The transition's own `when` ("resource.storageClassName == context.resource1.name")
# encodes exactly this: the PV that appears is the one whose storageClassName
# matches the created StorageClass -- i.e. it presupposes a PVC/Pod demand that
# names the class, which is outside operation `p`.
#
# Per CONTRIBUTING ("If you cannot make create-storageclass ALONE yield a PV,
# SKIP with that precise reason rather than faking it") this is a deliberate,
# honest SKIP: driving a PVC+Pod to force provisioning would prove a DIFFERENT
# transition (pod-create-reads-pvc already covers the consume->bind read), not
# "create StorageClass alone creates a PV".
#
# Spec: ../transitions/storageclass-create-creates-pv.json
#   when: resource.storageClassName == context.resource1.name
source "$(dirname "$0")/../lib.sh"
test_start "storageclass-create-creates-pv"

echo "RESULT: SKIP storageclass-create-creates-pv a StorageClass alone creates no PV; dynamic provisioning is provisioner-mediated and demand-driven (needs a PVC referencing the class, plus a consuming Pod under WaitForFirstConsumer, the mode of this cluster's default rancher.io/local-path class). create-storageclass ALONE never yields a PersistentVolume."
exit 3
