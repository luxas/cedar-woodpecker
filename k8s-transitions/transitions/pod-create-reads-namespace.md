# create Pod → get Namespace

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Low** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get namespaces`

## The implicit permission

Let *p* = `create pods` and *q* = `get namespaces`. Creating a namespaced object
causes the apiserver's admission chain to read the target Namespace object (to
confirm it exists and is not terminating). A principal holding *p* therefore
causes a read of the Pod's own Namespace without holding *q*. The edge is scoped
to the single Namespace the Pod lives in.

## Why it exists (Kubernetes mechanism)

The built-in `NamespaceExists` admission plugin fetches the request's Namespace
and rejects the create if it is absent, so every namespaced create implies a
Namespace read. (The `NamespaceAutoProvision` plugin does the same lookup before
optionally creating it.)

Source: [`exists.Admit` namespace lookup](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/admission/namespace/exists/admission.go#L70) — `_, err := e.namespaceLister.Get(a.GetNamespace())`, rejecting the request when the Namespace does not exist.

## Transition function

Uses derived field(s): none (relates `core::pods.namespace.name` to
`core::namespaces.name`).

`pod-create-reads-namespace.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-namespace",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::namespaces" },
  "when": "context.resource1.namespace.name == resource.name"
} ] }
```

## Example

Explicit policy (`pod-create-reads-namespace.cedar`):
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
// pod-create-reads-namespace: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::namespaces) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace.name == resource.name)
@woodpecker("pod-create-reads-namespace: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::namespaces
);
```

## Modeling notes

- `core::pods.namespace` is an ENTITY (`k8s::Namespace`), whereas
  `core::namespaces` is a different entity whose `.name` is the namespace string;
  they are related by `pod.namespace.name == ns.name`, per SCHEMA.md.
- This is a Low-severity, structural edge — the admission read is of the Pod's
  own namespace, not an arbitrary one, and a Namespace object exposes only
  labels/annotations/phase. It is listed for completeness of the create-Pod
  reconciliation fan-out.
