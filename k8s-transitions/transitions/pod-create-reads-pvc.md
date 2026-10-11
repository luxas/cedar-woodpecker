# create Pod → get PersistentVolumeClaim

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Medium** (classification.txt, `# medium` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get persistentvolumeclaims`

## The implicit permission

Let *p* = `create pods` and *q* = `get persistentvolumeclaims`. A principal
holding *p* can author a Pod with a `volumes[].persistentVolumeClaim.claimName`.
The kubelet/attach-detach path reads that PVC (and resolves the bound PV) to
mount the volume, so the Pod author causes a read of the named PVC without
holding *q*. The edge is scoped to the Pod's namespace and the PVCs it names.

## Why it exists (Kubernetes mechanism)

The Node authorizer graph walks the Pod's volumes and, for each
`persistentVolumeClaim`, adds a Pod→PVC edge (and from the PVC on to its bound PV
and that PV's Secrets). The node bound to the Pod may then read those PVCs. The
referenced claim names are the `core::pods.pvcRefs` derived field.

Source: [`Graph.AddPod` PVC loop](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L421) — iterates `pod.Spec.Volumes` and, for each PVC volume, calls `getOrCreateVertexLocked(pvcVertexType, pod.Namespace, claimName)` at [L428](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L428).

## Transition function

Uses derived field(s): `core::pods.pvcRefs` (see SCHEMA.md).

`pod-create-reads-pvc.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-pvc",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::persistentvolumeclaims" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.pvcRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-create-reads-pvc.cedar`):
```cedar
// Alice may create Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-create-reads-pvc: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::persistentvolumeclaims) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace == resource.namespace) && context.resource1.pvcRefs.contains(resource.name)
@woodpecker("pod-create-reads-pvc: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::persistentvolumeclaims
);
```

## Modeling notes

- The volume walk is projected to `core::pods.pvcRefs: Set<String>` (see
  SCHEMA.md); the `when` grants the implicit `get PVC` only for a named claim in
  the Pod's namespace.
- The real graph chains further (PVC → bound PV → that PV's Secrets); this file
  models only the first hop, Pod → PVC. The PV/Secret continuation is covered by
  the storage-family transitions.
- Severity Medium: a PVC object exposes capacity, storage class, and the bound
  PV name — useful for reconnaissance and for mounting another tenant's volume if
  the claim can be targeted.
