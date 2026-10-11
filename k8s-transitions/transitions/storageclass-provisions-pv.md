# create StorageClass → create PersistentVolume

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **Low** (classification.txt, the `# low` bucket; a cluster-scoped provisioning path that mints PVs on the principal's behalf).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create storageclasses --> create persistentvolumes`

## The implicit permission

Holding `create` on cluster-scoped `storageclasses` lets the principal define a
provisioner that, when a PVC names the class, causes a `PersistentVolume` to be
**dynamically created** by the PV controller / external provisioner — a
cluster-scoped object the principal was not granted write access to. Explicit
*p* = "create a StorageClass"; implicit *q* = "cause creation of a
PersistentVolume whose `storageClassName` is that class".

## Why it exists (Kubernetes mechanism)

Dynamic provisioning: when a PVC references a StorageClass by name, the PV
controller's `provisionClaimOperation` invokes the class's provisioner and then
creates a `PersistentVolume` object carrying `spec.storageClassName` equal to the
class name. External CSI provisioners follow the same contract. The volume is
cluster-scoped and is created by the controller/provisioner credentials, not the
StorageClass author's.

Source: [`PersistentVolumeController.provisionClaimOperation`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/volume/persistentvolume/pv_controller.go#L1567) — provisions a volume for a claim that names the StorageClass.
Source: [`PersistentVolumes().Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/volume/persistentvolume/pv_controller.go#L1703) — the controller creates the provisioned PV.

## Transition function

Uses derived field(s): `core::persistentvolumes.storageClassName` (Spec.StorageClassName,
see SCHEMA.md). The target PV is related to the source StorageClass by
`resource.storageClassName == context.resource1.name`.

`storageclass-create-creates-pv.json`:
```json
{ "transitions": [ {
  "name": "storageclass-create-creates-pv",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "io::k8s::storage::storageclasses" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::persistentvolumes" },
  "when": "resource.storageClassName == context.resource1.name"
} ] }
```

## Example

Explicit policy (`storageclass-create-creates-pv.cedar`):
```cedar
// A user who may create StorageClasses (cluster-scoped).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::storage::storageclasses
);
```

## Synthesized implicit policy

```
// storageclass-create-creates-pv: (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::storage::storageclasses) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::persistentvolumes) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.storageClassName == context.resource1.name)
@woodpecker("storageclass-create-creates-pv: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::persistentvolumes
);
```

## Modeling notes

- **Precise relation via derived field.** Unlike the APIService case, the schema
  exposes `persistentvolumes.storageClassName`, so the transition expresses the
  exact binding (`PV.storageClassName == StorageClass.name`) rather than a
  blanket `true`. The synthesized permit therefore scopes to PVs of this class.
- **Indirection through a PVC.** The real trigger is a PVC naming the class; the
  StorageClass author does not create the PVC. The transition models the terminal
  consequence (a PV of this class comes into existence) rather than the two-step
  PVC→PV chain, which is the capability the paper flags: a StorageClass author
  influences PV creation parameters (provisioner, mount options, reclaim policy).
- **Severity Low** (`classification.txt` `# low` bucket). PVs are cluster-scoped and provisioning can consume backend
  storage, but the author controls provisioning parameters rather than reading
  another tenant's data; the risk is resource/parameter abuse and reclaim-policy
  surprises.
