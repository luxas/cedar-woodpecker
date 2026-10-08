# create Namespace → create ServiceAccount / ConfigMap (default-object seeding)

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **Low** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create namespaces --> create serviceaccounts`
- `create namespaces --> create configmaps`

## The implicit permission

A principal who holds only `create` on `core::namespaces` (permission *p*) causes
two objects to be created inside the new namespace that they were never granted
`create` on (permission *q*): the `default` ServiceAccount and the
`kube-root-ca.crt` ConfigMap. The principal does not issue these creates — two
cluster controllers do, in reaction to the Namespace existing — but the objects
would not exist without the authorized `create namespaces` call, so the create
authority is implicitly conferred.

## Why it exists (Kubernetes mechanism)

Each new namespace is seeded with baseline objects by dedicated controllers that
watch namespace add/update events:

- The **ServiceAccounts controller** ensures a set of ServiceAccounts (by default
  just `default`) exists in every namespace, creating any that are missing on
  each namespace sync.
- The **root-ca-cert-publisher** creates the `kube-root-ca.crt` ConfigMap in
  every namespace so in-cluster clients can find the API server's CA bundle.

Source: [`ServiceAccountsController.syncNamespace`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/serviceaccount/serviceaccounts_controller.go#L202) — iterates `serviceAccountsToEnsure` (default: the `default` SA, L221) and creates each missing one in the namespace.
Source: [`Publisher.syncNamespace`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/certificates/rootcacertpublisher/publisher.go#L191) — on cache-miss it creates the [`kube-root-ca.crt`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/certificates/rootcacertpublisher/publisher.go#L201) ConfigMap in the namespace.

## Transition function

Uses derived field(s): none. (Relates `core::namespaces.name` to the created
object's `namespace.name`.)

`namespace-create-seeds-serviceaccount.json`:
```json
{ "transitions": [ {
  "name": "namespace-create-seeds-serviceaccount",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::namespaces" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::serviceaccounts" },
  "when": "resource.namespace.name == context.resource1.name"
} ] }
```

`namespace-create-seeds-configmap.json` differs only in `name` and
`target.resource` (`core::configmaps`).

## Example

Explicit policy (`namespace-create-seeds-serviceaccount.cedar`):
```cedar
// A user who may create namespaces (e.g. a self-service onboarding role).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::namespaces
);
```

## Synthesized implicit policy

`namespace-create-seeds-serviceaccount`:
```
// namespace-create-seeds-serviceaccount: (principal: k8s::User, action: k8s::Action::"create", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::serviceaccounts) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-create-seeds-serviceaccount: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::serviceaccounts
);
```

`namespace-create-seeds-configmap`:
```
// namespace-create-seeds-configmap: (principal: k8s::User, action: k8s::Action::"create", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-create-seeds-configmap: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::configmaps
);
```

## Modeling notes

- **Why the synthesized permit drops the namespace guard.** The source
  `create namespaces` request has no `namespace` of its own (Namespaces are
  cluster-scoped), so `context.resource1.name` is the *new* namespace's name,
  unconstrained by the explicit policy. Woodpecker therefore soundly collapses
  `resource.namespace.name == context.resource1.name` to `true` over the target
  environment — the principal gains `create` of the seeded object in *whatever*
  namespace they just created. This is the correct over-approximation: the
  authority follows every namespace they may create.
- **Over-approximation of the object set.** The real controllers create specific
  objects (`default` SA, `kube-root-ca.crt` ConfigMap). The schema has no `.name`
  constraint on the target in the trigger row, so the transition models "a
  ServiceAccount / a ConfigMap in the namespace" rather than those two names. A
  future admission policy could tighten the target to those literal names.
- **Severity (Low).** The seeded objects are benign baseline infrastructure and
  their creation is a normal, expected consequence of namespace creation. The
  implicit permission is catalogued for completeness; it confers no escalation
  beyond standard namespace bootstrapping.
