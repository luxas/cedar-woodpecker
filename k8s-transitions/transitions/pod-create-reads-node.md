# create Pod → get Node

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Medium** (classification.txt, `# medium` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get nodes`

Sibling rows handled separately: `create pods --> get namespaces` has its own
file (`pod-create-reads-namespace.md`); `create pods --> create podschedulingcontexts`
is `NOT SYNTHESIZABLE` here and is documented in
`pod-create-podschedulingcontexts.md`.

## The implicit permission

Let *p* = `create pods` and *q* = `get nodes`. A principal holding *p* can set
`spec.nodeName` directly (bypassing the scheduler) or let the scheduler assign
one. Either way a Node object is read to place the Pod, so the author causes a
read of a Node without holding *q*. This transition models the author-controlled
case where `spec.nodeName` names the Node.

## Why it exists (Kubernetes mechanism)

The Node authorizer graph gives every Pod an edge to its assigned Node
(`pod.Spec.NodeName`), which is how the scheduler/kubelet relate a Pod to the
Node that runs it. The name is the `core::pods.nodeName` derived field. A Node
is cluster-scoped, so the implicit `get` is not namespace-qualified.

Source: [`Graph.AddPod` node edge](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — `nodeVertex := g.getOrCreateVertexLocked(nodeVertexType, "", pod.Spec.NodeName)` at [L371](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L371).

## Transition function

Uses derived field(s): `core::pods.nodeName` (see SCHEMA.md).

`pod-create-reads-node.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-node",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::nodes" },
  "when": "context.resource1.nodeName == resource.name"
} ] }
```

## Example

Explicit policy (`pod-create-reads-node.cedar`):
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
// pod-create-reads-node: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::nodes) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.nodeName == resource.name)
@woodpecker("pod-create-reads-node: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::nodes
);
```

## Modeling notes

- Node is cluster-scoped (no `namespace` attribute), so the `when` has no
  namespace clause — it is a bare name match on the scalar
  `core::pods.nodeName`.
- The precise mechanism is the Node authorizer edge; the paper's severity Medium
  reflects that Node objects expose host IPs, capacity, labels and taints
  (reconnaissance), and that a principal able to pin `spec.nodeName` can bypass
  the scheduler and co-locate on a chosen Node.
