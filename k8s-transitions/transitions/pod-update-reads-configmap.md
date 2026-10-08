# update/patch Pod → get ConfigMap

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload mutation); severity **Medium** (classification.txt; the ConfigMap read is Medium, cf. the Critical Secret twin in `# Critical`).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update pods --> get configmaps`
- `patch pods --> get configmaps`

`update` and `patch` are the same mutation mechanism (one source), so they share
this file.

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

- Single `update` source subsumes the `patch pods --> get configmaps` row; the
  example policy permits both verbs.
- Same projection as the create twin: `core::pods.configMapRefs`, confined to the
  Pod's namespace.
- Severity Medium: ConfigMaps hold configuration, not credentials by design;
  listed here to complete the mutation-path fan-out alongside the Critical Secret
  variant.
