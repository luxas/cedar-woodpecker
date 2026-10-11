# delete Namespace → delete \<namespaced resource\> (namespace deletion cascade)

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **High (DoS)** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`) — every row below is this one mechanism:
- `delete namespaces --> delete pods`
- `delete namespaces --> delete deployments`
- `delete namespaces --> delete statefulsets`
- `delete namespaces --> delete daemonsets`
- `delete namespaces --> delete jobs`
- `delete namespaces --> delete cronjobs`
- `delete namespaces --> delete services`
- `delete namespaces --> delete configmaps`
- `delete namespaces --> delete secrets`
- `delete namespaces --> delete persistentvolumeclaims`
- `delete namespaces --> delete replicasets`
- `delete namespaces --> delete replicationcontrollers`
- `delete namespaces --> delete ingresses`
- `delete namespaces --> delete networkpolicies`
- `delete namespaces --> delete roles`
- `delete namespaces --> delete rolebindings`
- `delete namespaces --> delete serviceaccounts`
- `delete namespaces --> delete limitranges`
- `delete namespaces --> delete resourcequotas`
- `delete namespaces --> delete events`
- `delete namespaces --> delete endpoints`
- `delete namespaces --> delete horizontalpodautoscalers`
- `delete namespaces --> delete poddisruptionbudgets`
- `delete namespaces --> delete leases`
- `delete namespaces --> delete controllerrevisions`

## The implicit permission

A principal who holds only `delete` on `core::namespaces` for a namespace `n`
(permission *p*) can thereby cause the deletion of *every* namespaced object in
`n` — all pods, secrets, deployments, roles, PVCs, and so on (permission *q*),
none of which was granted to them. Deleting the Namespace object is a single
authorized call; the namespace controller then performs the cascade on the
caller's behalf, with no per-resource authorization check against the original
principal. Holding *p* over a namespace is therefore equivalent to holding
`deletecollection` over every resource type scoped to that namespace, which is
why the paper rates it High with a denial-of-service impact.

## Why it exists (Kubernetes mechanism)

Deleting a Namespace does not delete its contents synchronously. The API server
marks the Namespace `Terminating` and the **namespace controller** drives the
cascade: it enumerates every API resource (via discovery) that is namespaced and
`deletecollection`-capable and deletes the whole collection in that namespace,
looping until the namespace holds no remaining content, then clears the
`kubernetes` finalizer so the Namespace object itself is removed.

Source: [`namespacedResourcesDeleter.deleteAllContent`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/namespace/deletion/namespaced_resources_deleter.go#L501) — enumerates all namespaced group-versions and deletes each resource's collection in the terminating namespace.
Source: [`namespacedResourcesDeleter.deleteCollection`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/namespace/deletion/namespaced_resources_deleter.go#L309) — issues the actual `DeleteCollection` call per group-version-resource; this is the single code path every row above flows through, with the target entity varying only by `gvr`.
Source: [`namespacedResourcesDeleter.Delete`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/namespace/deletion/namespaced_resources_deleter.go#L96) — the controller entry point invoked once the Namespace enters `Terminating`.

## Transition function

Uses derived field(s): none. (Relates the cluster-scoped `core::namespaces.name`
to each namespaced entity's `namespace.name`.)

A representative set of four runnable pairs is shipped —
`namespace-delete-deletes-pods`, `-secrets`, `-configmaps`, `-deployments`. The
remaining rows are covered by the identical transition shape with only the
`target.resource` entity swapped (and `apps::`/`batch::`/`policy::`/
`io::k8s::…`/`autoscaling::` namespace prefixes as appropriate), because the
cross-type match is always `resource.namespace.name == context.resource1.name`.

`namespace-delete-deletes-pods.json`:
```json
{ "transitions": [ {
  "name": "namespace-delete-deletes-pods",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"delete\"", "resource": "core::namespaces" } ],
  "target":  { "action": "k8s::Action::\"delete\"", "resource": "core::pods" },
  "when": "resource.namespace.name == context.resource1.name"
} ] }
```

The three siblings differ only in `name` and `target.resource`
(`core::secrets`, `core::configmaps`, `apps::deployments`).

## Example

Explicit policy (`namespace-delete-deletes-pods.cedar`):
```cedar
// A user who may delete namespaces (e.g. a tenant admin over "team-a").
permit (
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::namespaces
)
when { resource.name == "team-a" };
```

## Synthesized implicit policy

`namespace-delete-deletes-pods`:
```
// namespace-delete-deletes-pods: (principal: k8s::User, action: k8s::Action::"delete", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"delete") && (context.resource1.name == "team-a") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-delete-deletes-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::pods
) when {
  resource.namespace.name == "team-a"
};
```

`namespace-delete-deletes-secrets`:
```
// namespace-delete-deletes-secrets: (principal: k8s::User, action: k8s::Action::"delete", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"delete") && (context.resource1.name == "team-a") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-delete-deletes-secrets: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::secrets
) when {
  resource.namespace.name == "team-a"
};
```

`namespace-delete-deletes-configmaps`:
```
// namespace-delete-deletes-configmaps: (principal: k8s::User, action: k8s::Action::"delete", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"delete") && (context.resource1.name == "team-a") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-delete-deletes-configmaps: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::configmaps
) when {
  resource.namespace.name == "team-a"
};
```

`namespace-delete-deletes-deployments`:
```
// namespace-delete-deletes-deployments: (principal: k8s::User, action: k8s::Action::"delete", resource: core::namespaces) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: apps::deployments) [sound]
// path: (context.action1 == k8s::Action::"delete") && (context.resource1.name == "team-a") && (resource.namespace.name == context.resource1.name)
@woodpecker("namespace-delete-deletes-deployments: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is apps::deployments
) when {
  resource.namespace.name == "team-a"
};
```

## Modeling notes

- **Cluster-scoped vs namespaced.** `core::namespaces` is cluster-scoped: its
  `.name` *is* the namespace string. The target entities are namespaced and
  carry `namespace: k8s::Namespace`, so the cross-type match is
  `resource.namespace.name == context.resource1.name` — the whole-entity form
  `resource.namespace == context.resource1` would not typecheck because the two
  sides are different entity types.
- **Over-approximation.** The transition's `when` constrains the cascade to the
  *same namespace name* and nothing finer. The real controller additionally only
  deletes resources that exist and are `deletecollection`-capable at cascade
  time; woodpecker (which synthesizes a permit, not the controller's runtime set)
  cannot see existence, so the synthesized permit is a sound over-approximation:
  it permits deleting any object of that type in the namespace, which is exactly
  the authority the cascade confers.
- **Severity (High / DoS).** A single authorized `delete namespaces` call
  destroys all workloads, config, secrets, RBAC and quotas in the namespace —
  equivalent to `deletecollection` across every namespaced type at once, with no
  second authorization check. For a tenant with namespace-delete rights this is a
  total, irreversible wipe of the namespace's contents.
- **Representative-set rationale (HARD RULE 4).** The 25 rows are one controller
  reconciliation (`deleteCollection` over a varying `gvr`), so per the brief this
  is one mechanism / one markdown. Four runnable pairs across the two schema
  namespaces (`core::` and `apps::`) exercise the transition shape; the rest vary
  only `target.resource`.
