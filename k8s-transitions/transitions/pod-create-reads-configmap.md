# create Pod → get ConfigMap

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Medium** (classification.txt, `# medium` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get configmaps`

## The implicit permission

Let *p* = `create pods` and *q* = `get configmaps`. A principal holding *p* can
author a Pod whose spec references a ConfigMap by name (an `env`/`envFrom`
`configMapKeyRef`, a `volumes[].configMap`, or a projected source). The kubelet
mounts that ConfigMap's contents into the container, so the Pod author reads it
without holding *q*. The edge is scoped to the Pod's namespace and the
ConfigMaps it names.

## Why it exists (Kubernetes mechanism)

The Node authorizer graph adds a Pod→ConfigMap edge for every ConfigMap the Pod
references, mirroring the Secret case. The referenced names come from
`VisitPodConfigmapNames`, and the kubelet reads them to populate the container.

Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — adds the pod→configmap edges (the ConfigMap loop around [L411](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L411)).
Source: [`VisitPodConfigmapNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L204) — enumerates every ConfigMap name a PodSpec references; this defines the `core::pods.configMapRefs` derived field.

## Transition function

Uses derived field(s): `core::pods.configMapRefs` (see SCHEMA.md).

`pod-create-reads-configmap.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-configmap",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::configmaps" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.configMapRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-create-reads-configmap.cedar`):
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
// pod-create-reads-configmap: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace == resource.namespace) && context.resource1.configMapRefs.contains(resource.name)
@woodpecker("pod-create-reads-configmap: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- The nested spec walk is projected to `core::pods.configMapRefs: Set<String>`
  (see SCHEMA.md); the `when` grants the implicit `get ConfigMap` only for a
  named ConfigMap in the Pod's namespace.
- Severity Medium (not Critical like the Secret twin): ConfigMaps are
  non-confidential by design, so the leak is of configuration data rather than
  credentials — though ConfigMaps are frequently misused to hold secrets, which
  is why the edge is not Low.
