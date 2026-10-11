# update/patch `<X>/status` → update/patch `<X>`

**Paper:** Yang et al., CCS '25 (implicit permissions of Kubernetes subresources); severity **Medium** per the paper's generic treatment of `/status`, but the triggered-row buckets in `classification.txt` are **split**: most `update/patch <X>/status` rows land in **low**, while `update/patch nodes/status` is **Critical** (a Node's status drives scheduling, eviction, and the node-lease/taint machinery). The real bucket is reported per row below.

Reference(s) formalized (from `assessment/trigger.txt`) — the generic `X/status` family. Every one of these is `update <X>/status --> update <X>` **and** `patch <X>/status --> patch <X>` (create/update/patch of the same shared store are one mechanism):

- `update apiservices/status --> update apiservices` / `patch …` — low
- `update certificatesigningrequests/status --> update certificatesigningrequests` / `patch …` — low
- `update cronjobs/status --> update cronjobs` / `patch …` — low
- `update customresourcedefinitions/status --> update customresourcedefinitions` / `patch …` — low
- `update daemonsets/status --> update daemonsets` / `patch …` — low
- `update deployments/status --> update deployments` / `patch …` — low  *(runnable below)*
- `update flowschemas/status --> update flowschemas` / `patch …` — low
- `update horizontalpodautoscalers/status --> update horizontalpodautoscalers` / `patch …` — low
- `update ingresses/status --> update ingresses` / `patch …` — low
- `update jobs/status --> update jobs` / `patch …` — low
- `update namespaces/status --> update namespaces` / `patch …` — low
- `update nodes/status --> update nodes` / `patch …` — **Critical**  *(runnable below)*
- `update persistentvolumeclaims/status --> update persistentvolumeclaims` / `patch …` — low
- `update persistentvolumes/status --> update persistentvolumes` / `patch …` — low
- `update poddisruptionbudgets/status --> update poddisruptionbudgets` / `patch …` — low
- `update pods/status --> update pods` / `patch …` — low  *(runnable below)*
- `update podschedulingcontexts/status --> update podschedulingcontexts` / `patch …` — low
- `update prioritylevelconfigurations/status --> update prioritylevelconfigurations` / `patch …` — low
- `update replicasets/status --> update replicasets` / `patch …` — low
- `update replicationcontrollers/status --> update replicationcontrollers` / `patch …` — low
- `update resourceclaims/status --> update resourceclaims` / `patch …` — low
- `update resourcequotas/status --> update resourcequotas` / `patch …` — low  *(runnable below)*
- `update servicecidrs/status --> update servicecidrs` / `patch …` — low
- `update services/status --> update services` / `patch …` — low
- `update statefulsets/status --> update statefulsets` / `patch …` — low
- `update storageversionmigrations/status --> update storageversionmigrations` / `patch …` — low
- `update storageversions/status --> update storageversions` / `patch …` — low
- `update validatingadmissionpolicies/status --> update validatingadmissionpolicies` / `patch …` — low
- `update volumeattachments/status --> update volumeattachments` / `patch …` — low

The `pods/ephemeralcontainers` subresource write (`update/patch pods/ephemeralcontainers --> update/patch pods`) is the same shared-store mechanism; it is listed here but its runnable *get* companion lives in `get-status-reads-parent.md`.

## The implicit permission

Hold `p = update <X>/status` (write access to the `/status` subresource of some
object kind X) and you also hold `q = update <X>` for the **same named object**.
A `/status` write is not a write to a *separate* object: it writes the status
stanza of the very same stored object X, under the same storage key, with the
same `metadata`. An authorizer that grants only `/status` believes it has
withheld the ability to mutate the object — but the status write *is* a write to
the object. (The subresource strategy restricts *which fields* a `/status`
request may change to the status subset, but that is a field-level admission
concern, not an authorization boundary; the resource being written is X.)

## Why it exists (Kubernetes mechanism)

Every built-in resource's `/status` endpoint is served by a REST handler that
wraps the **same `genericregistry.Store`** as the parent resource. In the pod
storage wiring the status store is a *struct copy* of the parent store with only
the update/reset strategies swapped — same `KeyFunc`, same backend, same stored
object:

Source: [`NewStorage` pod wiring — `statusStore := *store`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L103) — the `/status` store copies the parent store and only replaces `UpdateStrategy`/`ResetFieldsStrategy` with `StatusStrategy`; the backend storage key is identical.

Source: [`StatusREST.Update`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L371) — a `/status` update calls straight through to `r.store.Update(...)` on that shared store, i.e. the parent object's own Update path.

Source: [generic `(*Store).Update`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go#L772) — the single update implementation reached by both the parent and its `/status` subresource; the subresource writes the one stored object.

Every other resource kind follows the identical pattern in its own
`pkg/registry/.../storage/storage.go` (a `statusStore := *store` copy plus a
`StatusREST`/`*statusStrategy`), which is why one mechanism subsumes the whole
list above.

## Transition function

Uses derived field(s): none. The match is purely by object identity
(`name` + `namespace`, or `name` alone for cluster-scoped kinds like Node).

Representative runnable files (one per shape — namespaced and cluster-scoped):

`pods-status-updates-parent.json`:
```json
{ "transitions": [ {
  "name": "pods-status-updates-parent",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::pods_status" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "core::pods" },
  "when": "resource.name == context.resource1.name && resource.namespace == context.resource1.namespace"
} ] }
```

`nodes-status-updates-parent.json` (cluster-scoped — `nodes_status` has **no**
`namespace` field, so the `when` matches on `name` alone):
```json
{ "transitions": [ {
  "name": "nodes-status-updates-parent",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::nodes_status" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "core::nodes" },
  "when": "resource.name == context.resource1.name"
} ] }
```

`deployments-status-updates-parent.json` and
`resourcequotas-status-updates-parent.json` are the same namespaced shape over
`apps::deployments_status`→`apps::deployments` and
`core::resourcequotas_status`→`core::resourcequotas` respectively.

## Example

Explicit policy (`pods-status-updates-parent.cedar`):
```cedar
// A user granted update on the pods/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::pods_status
)
when { resource.namespace.name == "team-a" };
```

## Synthesized implicit policy

`pods-status-updates-parent`:
```
// pods-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: core::pods_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("pods-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::pods
) when {
  resource.namespace.name == "team-a"
};
```

`nodes-status-updates-parent`:
```
// nodes-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: core::nodes_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: core::nodes) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("nodes-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::nodes
);
```

`deployments-status-updates-parent`:
```
// deployments-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: apps::deployments_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: apps::deployments) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("deployments-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is apps::deployments
) when {
  resource.namespace.name == "team-a"
};
```

`resourcequotas-status-updates-parent`:
```
// resourcequotas-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: core::resourcequotas_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: core::resourcequotas) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("resourcequotas-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::resourcequotas
) when {
  resource.namespace.name == "team-a"
};
```

All four reported `[sound]`.

## Modeling notes

- **One shape covers the whole list.** The transition carries no attribute of X
  beyond identity, so the exact same `when` (match on `name`, plus `namespace`
  when the kind is namespaced) is sound for every kind in the row list — only
  the two entity names change. We ship four representative pairs (`pods`,
  `nodes`, `deployments`, `resourcequotas`) rather than ~30 near-identical files;
  `nodes` is included specifically because it is the cluster-scoped shape (no
  `namespace`) *and* the one Critical row.
- **The `patch` rows are the same transition.** `patch <X>/status --> patch <X>`
  takes the identical shared store via the identical `StatusREST`; swapping the
  action from `"update"` to `"patch"` in both source and target yields a sound
  transition of the same shape, so we do not ship a separate `patch` file per
  kind.
- **No over-approximation in the condition itself.** Unlike the derived-field
  transitions (e.g. `secretRefs.contains(...)` which widens to a namespace
  restriction in the synthesized policy), here the identity match
  `resource.name == context.resource1.name` is exact: holding `/status` on an
  object is holding write on *that* object, nothing wider. The synthesized
  policy drops the `name` equality only because the example policy constrained
  the source by namespace, not by name — i.e. the user who may write *any*
  pod's status in `team-a` may write *any* pod in `team-a`, which is precisely
  the real escalation.
- **Severity split is real, not a modeling artifact.** `nodes/status` is
  Critical because a Node object's status feeds scheduling, eviction, and
  pressure/taint reactions; writing arbitrary node status can cordon or
  overload the cluster. The remaining kinds are low because their status is
  observational and reconciled back by their controller. We report the bucket
  from `classification.txt` per row rather than flattening to the paper's
  generic "Medium".
