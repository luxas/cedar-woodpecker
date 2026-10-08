# create Pod → get ServiceAccount

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Medium** (classification.txt, `# medium` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get serviceaccounts`

## The implicit permission

Let *p* = `create pods` and *q* = `get serviceaccounts`. A principal holding *p*
can set `spec.serviceAccountName` on the Pod it creates. The kubelet/apiserver
then reads that ServiceAccount to mint and project its token into the Pod, so
the author causes a read of the named ServiceAccount without holding *q*. The
edge is scoped to the Pod's namespace and the single SA it names.

## Why it exists (Kubernetes mechanism)

The Node authorizer graph adds a Pod→ServiceAccount edge whenever
`pod.Spec.ServiceAccountName` is non-empty, so the node bound to the Pod may read
that SA (and the apiserver reads it to project a bound token). The referenced
name is exactly `Spec.ServiceAccountName`, the `core::pods.serviceAccountRef`
derived field.

Source: [`Graph.AddPod` SA edge](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L385) — `if len(pod.Spec.ServiceAccountName) > 0 { … getOrCreateVertexLocked(serviceAccountVertexType, pod.Namespace, pod.Spec.ServiceAccountName) }`.

## Transition function

Uses derived field(s): `core::pods.serviceAccountRef` (see SCHEMA.md).

`pod-create-reads-serviceaccount.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-serviceaccount",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::serviceaccounts" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.serviceAccountRef == resource.name"
} ] }
```

## Example

Explicit policy (`pod-create-reads-serviceaccount.cedar`):
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
// pod-create-reads-serviceaccount: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::serviceaccounts) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace == resource.namespace) && (context.resource1.serviceAccountRef == resource.name)
@woodpecker("pod-create-reads-serviceaccount: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::serviceaccounts
);
```

## Modeling notes

- `serviceAccountRef` is a scalar `String` (one SA per Pod), so the `when` is an
  equality rather than a set membership. If the field is empty the Pod defaults
  to the `default` SA at admission; the projection models the explicitly-set
  name, which is the author-controlled case.
- Severity Medium: reading the SA object itself is less sensitive than reading
  its token Secret, but it is a stepping stone — pairing this with the SA's
  projected token is how Pod creation escalates to the SA's RBAC.
