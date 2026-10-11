# update Namespace/finalize → update / delete Namespace (finalizer-driven completion)

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity
**Critical** for `--> update namespaces`, **High** for `--> delete namespaces`
(classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update namespaces/finalize --> update namespaces`
- `update namespaces/finalize --> delete namespaces`

> Note: `update namespaces/status --> update namespaces` and
> `patch namespaces/status --> patch namespaces` also appear in `trigger.txt`.
> Those belong to the **status subresource family (Family F)** — the generic
> `/status` → parent pattern across all kinds — and are deferred there; this
> file covers only the `/finalize` subresource, whose effect (completing
> deletion) is specific to Namespaces.

## The implicit permission

A principal who holds only `update` on the `core::namespaces_finalize`
subresource (permission *p*) can mutate the parent Namespace object — both its
`spec.finalizers` and, as a direct consequence, its lifecycle. Writing `/finalize`
*is* a write to the Namespace (permission *q* = `update namespaces`), and because
removing the last finalizer from a `Terminating` Namespace is what lets the API
server actually delete it, the same permission yields `delete namespaces` in
effect (permission *q'*). The principal never needs `update`/`delete` on the
Namespace resource itself.

## Why it exists (Kubernetes mechanism)

`/finalize` is a subresource of Namespace whose sole job is to update
`spec.finalizers`. It is backed by a store that shares the Namespace's underlying
storage but uses the dedicated `FinalizeStrategy`, so a write to `/finalize` is a
write to the same stored Namespace object. The namespace REST layer treats a
Namespace that is `Terminating` and has an empty finalizer list as deletable:
`ShouldDeleteNamespaceDuringUpdate` returns true when the finalizers are gone, so
the very update that clears the last finalizer causes the API server to delete
the Namespace during that update.

Source: [`FinalizeREST.Update`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/namespace/storage/storage.go#L354) — the `/finalize` endpoint; it writes the Namespace via the shared store using `FinalizeStrategy`.
Source: [`FinalizeStrategy`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/namespace/strategy.go#L189) — the update strategy the finalize store swaps in, confirming `/finalize` mutates the parent Namespace's finalizers.
Source: [`ShouldDeleteNamespaceDuringUpdate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/namespace/storage/storage.go#L259) — returns true once the finalizer set is empty on a terminating Namespace, so the finalize update triggers deletion of the object.

## Transition function

Uses derived field(s): none. (Subresource → parent, matched by name; both are
cluster-scoped so there is no namespace to match.)

`namespace-finalize-updates-namespace.json`:
```json
{ "transitions": [ {
  "name": "namespace-finalize-updates-namespace",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::namespaces_finalize" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "core::namespaces" },
  "when": "resource.name == context.resource1.name"
} ] }
```

`namespace-finalize-deletes-namespace.json` differs only in `name` and the
`target.action` (`k8s::Action::"delete"`).

## Example

Explicit policy (`namespace-finalize-updates-namespace.cedar`):
```cedar
// A controller/user permitted to write the /finalize subresource of namespaces.
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::namespaces_finalize
);
```

## Synthesized implicit policy

`namespace-finalize-updates-namespace`:
```
// namespace-finalize-updates-namespace: (principal: k8s::User, action: k8s::Action::"update", resource: core::namespaces_finalize) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: core::namespaces) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("namespace-finalize-updates-namespace: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::namespaces
);
```

`namespace-finalize-deletes-namespace`:
```
// namespace-finalize-deletes-namespace: (principal: k8s::User, action: k8s::Action::"update", resource: core::namespaces_finalize) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::namespaces) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("namespace-finalize-deletes-namespace: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::namespaces
);
```

## Modeling notes

- **Cluster-scoped, name-matched.** Both `core::namespaces_finalize` and
  `core::namespaces` are cluster-scoped; their `.name` is the namespace string.
  The subresource→parent match is therefore `resource.name == context.resource1.name`
  with no namespace term (unlike the namespaced subresource pattern e.g.
  `pods_status`, which also matches on `.namespace`).
- **Why the synthesized permit drops the name guard.** The explicit policy puts
  no constraint on *which* namespace's `/finalize` the principal may write, so
  `context.resource1.name` is free and woodpecker soundly collapses the match to
  an unconditional `permit` over the target: write-finalize-anywhere ⇒
  update/delete-any-namespace.
- **Severity.** `--> update namespaces` is **Critical**: `/finalize` write access
  is a full write path to Namespace objects, bypassing any policy that gates the
  `namespaces` resource directly. `--> delete namespaces` is **High (DoS)**:
  clearing the last finalizer on a terminating Namespace completes its deletion,
  which in turn triggers the whole namespace-delete cascade documented in
  `namespace-delete-cascades.md`. So `/finalize` write ⇒ namespace delete ⇒
  cascade wipe of the namespace's contents.
- **Both transitions are sound and runnable** (no deny-side effect); both ship
  `.json`/`.cedar` pairs.
