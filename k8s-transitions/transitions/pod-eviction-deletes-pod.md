# create pods/eviction → delete Pod

**Paper:** Yang et al., CCS '25 §4 (subresource implicit permissions); severity **Critical** (classification.txt, `# Critical` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods/eviction --> delete pods`

## The implicit permission

Let *p* = `create pods/eviction` and *q* = `delete pods`. Posting an `Eviction`
to a Pod's `eviction` subresource causes the apiserver to delete that Pod
(subject to the Pod's PodDisruptionBudget). A principal holding only *p* can
therefore delete the Pod — the capability that *q* grants — without holding *q*.
The edge is scoped to the same Pod name and namespace.

## Why it exists (Kubernetes mechanism)

The eviction subresource is implemented by `EvictionREST`. Its `Create` handler,
after checking any applicable PodDisruptionBudget, calls the Pod store's `Delete`
to remove the Pod. So a successful `create` on `pods/eviction` is literally a
`delete` of the parent Pod.

Source: [`EvictionREST.Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/eviction.go#L130) — entry point for `POST .../eviction`.
Source: [the Pod store `Delete` call](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/eviction.go#L382) — `deletedObj, _, err := r.store.Delete(ctx, name, rest.ValidateAllObjectFunc, options)`, the actual deletion of the parent Pod.

## Transition function

Uses derived field(s): none (subresource→parent name+namespace match).

`pod-eviction-deletes-pod.json`:
```json
{ "transitions": [ {
  "name": "pod-eviction-deletes-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods_eviction" } ],
  "target":  { "action": "k8s::Action::\"delete\"", "resource": "core::pods" },
  "when": "context.resource1.name == resource.name && context.resource1.namespace == resource.namespace"
} ] }
```

## Example

Explicit policy (`pod-eviction-deletes-pod.cedar`):
```cedar
// Alice may create eviction subresources on Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods_eviction
);
```

## Synthesized implicit policy

```
// pod-eviction-deletes-pod: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods_eviction) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.name == resource.name) && (context.resource1.namespace == resource.namespace)
@woodpecker("pod-eviction-deletes-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::pods
);
```

## Modeling notes

- A subresource→parent transition keyed on name + namespace (the known-good
  "subresource → parent" shape in BRIEF.md), with the verb crossing from
  `create` on the subresource to `delete` on the parent Pod.
- Over-approximation: the real deletion is gated by a PodDisruptionBudget, which
  can make eviction fail. That is a *deny-side* refinement the authorizer does
  not see; woodpecker synthesizes the permit (the capability), so the sound
  statement is "create eviction can delete the same-named Pod". The PDB gate
  would be modeled as a separate deny policy, not here.
- Severity Critical: unauthenticated-of-`delete` Pod availability impact — a
  principal with only eviction rights can tear down workloads.
