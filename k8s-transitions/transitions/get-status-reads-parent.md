# get `<X>/status` → get `<X>`  (and `get pods/ephemeralcontainers → get pods`)

**Paper:** Yang et al., CCS '25 (implicit permissions of Kubernetes subresources); severity **Medium** (information disclosure) — confirmed by `classification.txt`, where every row below sits in the `# medium` bucket.

Reference(s) formalized (from `assessment/trigger.txt`) — the generic `get <X>/status --> get <X>` family (all **medium**):

- `get horizontalpodautoscalers/status --> get horizontalpodautoscalers`
- `get ingresses/status --> get ingresses`  *(runnable below)*
- `get jobs/status --> get jobs`  *(runnable below)*
- `get persistentvolumeclaims/status --> get persistentvolumeclaims`
- `get poddisruptionbudgets/status --> get poddisruptionbudgets`
- `get pods/status --> get pods`  *(runnable below)*
- `get podschedulingcontexts/status --> get podschedulingcontexts`
- `get prioritylevelconfigurations/status --> get prioritylevelconfigurations`
- `get replicasets/status --> get replicasets`
- `get replicationcontrollers/status --> get replicationcontrollers`
- `get servicecidrs/status --> get servicecidrs`
- `get statefulsets/status --> get statefulsets`
- `get storageversions/status --> get storageversions`
- `get validatingadmissionpolicies/status --> get validatingadmissionpolicies`
- `get volumeattachments/status --> get volumeattachments`

Plus the sibling subresource read, same mechanism:

- `get pods/ephemeralcontainers --> get pods`  *(runnable below)*

## The implicit permission

Hold `p = get <X>/status` and you also hold `q = get <X>`. A `GET` on the
`/status` subresource does **not** return a status-only projection — it returns
the entire stored object, including `metadata`, `spec`, and the full `status`.
There is no field-level narrowing on read. So a principal granted only
`get <X>/status` can read every field of X, which is exactly what `get <X>`
grants: the two differ in URL, not in the bytes returned. The paper rates this
Medium as an information-disclosure escalation (a reader scoped to "status only"
in fact sees the whole object — specs, annotations, references).

## Why it exists (Kubernetes mechanism)

The `/status` GET handler is the parent object's own `Get` on the **shared
store**. In pod storage, `StatusREST.Get` forwards verbatim to `r.store.Get`,
and that store is a struct copy of the parent store (same key, same object):

Source: [`StatusREST.Get`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L366) — "It is required to support Patch"; it returns `r.store.Get(ctx, name, options)`, i.e. the whole object, with no status-only filtering.

Source: [`NewStorage` — `statusStore := *store`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L103) — the status store copies the parent store; the read strategy is not narrowed on GET, only the *update* strategy is swapped.

For `pods/ephemeralcontainers` the same wiring applies via a parallel copy:

Source: [`NewStorage` — `ephemeralContainersStore := *store`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L110) — the ephemeral-containers subresource is another struct copy of the pod store; a read of it is a read of the whole pod object.

## Transition function

Uses derived field(s): none — object-identity match only.

`pods-status-reads-parent.json`:
```json
{ "transitions": [ {
  "name": "pods-status-reads-parent",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"get\"", "resource": "core::pods_status" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::pods" },
  "when": "resource.name == context.resource1.name && resource.namespace == context.resource1.namespace"
} ] }
```

`ingresses-status-reads-parent.json` and `jobs-status-reads-parent.json` are the
same namespaced shape over `io::k8s::networking::ingresses_status`→`…::ingresses`
and `batch::jobs_status`→`batch::jobs`. `pods-ephemeralcontainers-reads-parent.json`
targets `core::pods` from `core::pods_ephemeralcontainers`.

## Example

Explicit policy (`pods-status-reads-parent.cedar`):
```cedar
// A user granted get on the pods/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::pods_status
)
when { resource.namespace.name == "team-a" };
```

## Synthesized implicit policy

`pods-status-reads-parent`:
```
// pods-status-reads-parent: (principal: k8s::User, action: k8s::Action::"get", resource: core::pods_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"get") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("pods-status-reads-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::pods
) when {
  resource.namespace.name == "team-a"
};
```

`ingresses-status-reads-parent`:
```
// ingresses-status-reads-parent: (principal: k8s::User, action: k8s::Action::"get", resource: io::k8s::networking::ingresses_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: io::k8s::networking::ingresses) [sound]
// path: (context.action1 == k8s::Action::"get") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("ingresses-status-reads-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is io::k8s::networking::ingresses
) when {
  resource.namespace.name == "team-a"
};
```

`jobs-status-reads-parent`:
```
// jobs-status-reads-parent: (principal: k8s::User, action: k8s::Action::"get", resource: batch::jobs_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: batch::jobs) [sound]
// path: (context.action1 == k8s::Action::"get") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("jobs-status-reads-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is batch::jobs
) when {
  resource.namespace.name == "team-a"
};
```

`pods-ephemeralcontainers-reads-parent`:
```
// pods-ephemeralcontainers-reads-parent: (principal: k8s::User, action: k8s::Action::"get", resource: core::pods_ephemeralcontainers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"get") && (context.resource1.namespace.name == "team-a") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("pods-ephemeralcontainers-reads-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::pods
) when {
  resource.namespace.name == "team-a"
};
```

All four reported `[sound]`.

## Modeling notes

- **Same object-identity transition as the write family**, just with
  `k8s::Action::"get"` on both sides. One shape covers every row in the list;
  we ship three representative `/status` reads plus the `/ephemeralcontainers`
  read, differing only in entity names.
- **The escalation is read-amplification, not write.** The synthesized policy
  shows a reader scoped to `/status` in `team-a` can `get` the full parent in
  `team-a` — i.e. "status-only" visibility is in fact full-object visibility.
  This is why the paper buckets it medium (info leak) rather than low: specs,
  annotations, and object references (image names, mounted volume/secret names
  in metadata) all come back.
- **`scale`-read rows:** `trigger.txt` contains **no** `get <X>/scale -->
  get <X>` rows, so there is nothing to formalize for scale reads here (see
  `scale-reads-parent.md`). Scale *writes* that change replica counts are owned
  by Family B and are deliberately not duplicated.
- The condition is exact (identity match); the synthesized policy drops the
  `name` equality only because the example scopes the source by namespace, not
  by a single name.
