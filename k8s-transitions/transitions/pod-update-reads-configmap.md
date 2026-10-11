# update Pod → get ConfigMap

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload mutation); severity **Medium** (classification.txt; the ConfigMap read is Medium, cf. the Critical Secret twin in `# Critical`).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update pods --> get configmaps`

The sibling row `patch pods --> get configmaps` is filed separately in
[`pod-patch-image-reads-configmap.md`](pod-patch-image-reads-configmap.md): `patch`
has a *reachable* exploit (swap the container image to run attacker code against the
already-mounted ConfigMaps) that this add-a-ref path does not. Both verbs can attempt
either behaviour; each trigger row is filed once under its canonical mechanism.

## The implicit permission

Let *p* = `update pods` (or `patch pods`) and *q* = `get configmaps`. A principal
holding *p* can mutate a Pod to reference a ConfigMap by name; the kubelet mounts
that ConfigMap, so the author reads it without holding *q*. The edge is scoped to
the Pod's namespace and the ConfigMaps it names.

## Why it exists (Kubernetes mechanism)

Identical capability path to create: the Node authorizer graph rebuilds the
Pod→ConfigMap edges from the current Pod, and the kubelet mounts the referenced
ConfigMaps.

Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — rebuilds the pod→configmap edges (ConfigMap loop at [L411](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L411)).
Source: [`VisitPodConfigmapNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L204) — enumerates the referenced ConfigMap names (the `core::pods.configMapRefs` derived field).

## Transition function

Uses derived field(s): `core::pods.configMapRefs` (see SCHEMA.md).

`pod-update-reads-configmap.json`:
```json
{ "transitions": [ {
  "name": "pod-update-reads-configmap",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::configmaps" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.configMapRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-update-reads-configmap.cedar`):
```cedar
// Alice may update (and patch) Pods in any namespace.
permit (
  principal is k8s::User,
  action in [k8s::Action::"update", k8s::Action::"patch"],
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-update-reads-configmap: (principal: k8s::User, action: k8s::Action::"update", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.namespace == resource.namespace) && context.resource1.configMapRefs.contains(resource.name)
@woodpecker("pod-update-reads-configmap: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- The transition's single source uses `update`; the `patch pods --> get configmaps`
  row is filed under
  [`pod-patch-image-reads-configmap.md`](pod-patch-image-reads-configmap.md), whose
  image-swap exploit is reachable where this add-a-ref path is not. The example
  policy permits both verbs.
- Same projection as the create twin: `core::pods.configMapRefs`, confined to the
  Pod's namespace.
- Severity Medium: ConfigMaps hold configuration, not credentials by design;
  listed here to complete the mutation-path fan-out alongside the Critical Secret
  variant.
