# patch Pod (image swap) → get ConfigMap

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload mutation); also
DataDog **KubeHound** `POD_PATCH` ("Patch running pod" → *Container Administration
Command* → *Execution*). Severity **Medium** (classification.txt; the ConfigMap read
is Medium, cf. the Critical Secret twin `pod-patch-image-reads-secret`).
Reference(s) formalized (from `assessment/trigger.txt`):
- `patch pods --> get configmaps`

## The implicit permission

Let *p* = `patch pods` and *q* = `get configmaps`. A principal holding *p* can
replace the `image` of a running Pod's container with an attacker-controlled
image. The kubelet recreates the container in place, re-using the Pod's existing
ConfigMap volume mounts, so attacker code runs with those ConfigMap bytes on its
filesystem and reads every ConfigMap the Pod mounts without the principal holding
*q*. The edge is scoped to the Pod's namespace and the ConfigMaps it already
references (`configMapRefs`).

This is the ConfigMap analogue of
[`pod-patch-image-reads-secret.md`](pod-patch-image-reads-secret.md), and the
*reachable* sibling of [`pod-update-reads-configmap.md`](pod-update-reads-configmap.md):
adding a *new* ConfigMap reference by mutation is rejected by `ValidatePodUpdate`
(Pod spec immutability — see that file's honest SKIP), whereas the image swap here
needs no new reference; it reads the ConfigMaps the Pod *already* mounts.

## Why it exists (Kubernetes mechanism)

`ValidatePodUpdate` forbids changing almost the entire Pod spec after creation,
but **`spec.containers[*].image`** is on its small allowlist of updatable fields.
Replacing the image is a legal `update`/`patch`, and the kubelet reconciles the
container by recreating it from the new image while leaving the Pod's volumes —
including its ConfigMap volumes — mounted into the replacement.

Source: [`updatablePodSpecFields`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/apis/core/validation/validation.go#L5862) — the allowlist naming `spec.containers[*].image` as a mutable field.
Source: [`ValidatePodUpdate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/apis/core/validation/validation.go#L5875) — enforces that only those fields change on a Pod update/patch.
Source: [`VisitPodConfigmapNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L204) — enumerates the ConfigMap names the kubelet mounts (the `core::pods.configMapRefs` derived field), unchanged by an image swap.
Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — the pod→configmap edges (ConfigMap loop at [L411](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L411)) whose mounted targets the swapped-in container can read.

## Transition function

Uses derived field(s): `core::pods.configMapRefs` (see SCHEMA.md) — the ConfigMaps
the Pod already mounts, which the swapped-in image inherits.

`pod-patch-image-reads-configmap.json`:
```json
{ "transitions": [ {
  "name": "pod-patch-image-reads-configmap",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"patch\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::configmaps" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.configMapRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-patch-image-reads-configmap.cedar`):
```cedar
// Alice may patch Pods in any namespace (e.g. to roll a container image).
permit (
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-patch-image-reads-configmap: (principal: k8s::User, action: k8s::Action::"patch", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"patch") && (context.resource1.namespace == resource.namespace) && context.resource1.configMapRefs.contains(resource.name)
@woodpecker("pod-patch-image-reads-configmap: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- Bounded by `configMapRefs` (the Pod's already-mounted set) plus the namespace
  guard — exactly what the swapped-in image can read. Same shape as the Secret
  twin, differing only in target resource / derived field.
- The `patch pods --> get configmaps` trigger row is filed here;
  `pod-update-reads-configmap` keeps the `update pods --> get configmaps` row and
  documents the (blocked) add-a-ref path.
- Severity Medium: ConfigMaps hold configuration, not credentials by design; the
  Critical consequence of the same image swap is the Secret / SA-token read
  (`pod-patch-image-reads-secret`).
