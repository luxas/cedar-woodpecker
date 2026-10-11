# status subresource → parent (networking & storage: servicecidrs, volumeattachments)

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **Medium** (classification.txt — a status-subresource write that reaches the parent object, both cluster-scoped).
Reference(s) formalized (from `assessment/trigger.txt`):
- `get volumeattachments/status --> get volumeattachments`
- `update volumeattachments/status --> update volumeattachments`
- `patch volumeattachments/status --> patch volumeattachments`
- `get servicecidrs/status --> get servicecidrs`
- `update servicecidrs/status --> update servicecidrs`
- `patch servicecidrs/status --> patch servicecidrs`

(The generic status subresource store shares the parent's REST store; get/update/patch
on `/status` are the same object as the parent, so each resource folds its verb
family into one subresource→parent transition.)

## The implicit permission

Holding a mutating verb on the `/status` subresource of a cluster-scoped object
(`volumeattachments/status`, `servicecidrs/status`) lets the principal write the
status stanza of the **same-named parent object**. Because the subresource and
the parent are one stored object, status access is effectively write access to
part of the parent the principal was not granted directly. Explicit *p* = "update
`<kind>/status`"; implicit *q* = "update the parent `<kind>` of the same name".

## Why it exists (Kubernetes mechanism)

A status subresource is implemented by cloning the parent's generic REST store
and swapping in a status-only update strategy; `/status` and the main endpoint
address the *same* stored object (same name), differing only in which fields the
strategy permits writing. So a principal with status write can mutate the parent
object's status.

Source: [`VolumeAttachment NewStorage` (statusStore = *store)](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/storage/volumeattachment/storage/storage.go#L68) — the status REST is a copy of the parent store with the status strategy.
Source: [`ServiceCIDR status REST`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/networking/servicecidr/storage/storage.go#L62) — the ServiceCIDR status store reuses the parent store with `servicecidr.StatusStrategy`.

## Transition function

Uses derived field(s): none. Both resources are cluster-scoped, so the
subresource→parent relation is name equality alone:
`resource.name == context.resource1.name` (no namespace term — these entities
have no `namespace` attribute).

`volumeattachment-status-updates-parent.json`:
```json
{ "transitions": [ {
  "name": "volumeattachment-status-updates-parent",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "io::k8s::storage::volumeattachments_status" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "io::k8s::storage::volumeattachments" },
  "when": "resource.name == context.resource1.name"
} ] }
```

`servicecidr-status-updates-parent.json`:
```json
{ "transitions": [ {
  "name": "servicecidr-status-updates-parent",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "io::k8s::networking::servicecidrs_status" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "io::k8s::networking::servicecidrs" },
  "when": "resource.name == context.resource1.name"
} ] }
```

## Example

Explicit policies (`volumeattachment-status-updates-parent.cedar`, `servicecidr-status-updates-parent.cedar`):
```cedar
// A user (e.g. a CSI attacher principal) who may update VolumeAttachment status
// (cluster-scoped storage subresource).
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::storage::volumeattachments_status
);
```
```cedar
// A user who may update ServiceCIDR status (cluster-scoped networking subresource).
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::networking::servicecidrs_status
);
```

## Synthesized implicit policy

```
// volumeattachment-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::storage::volumeattachments_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::storage::volumeattachments) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("volumeattachment-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::storage::volumeattachments
);
```

```
// servicecidr-status-updates-parent: (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::networking::servicecidrs_status) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::networking::servicecidrs) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("servicecidr-status-updates-parent: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::networking::servicecidrs
);
```

## Modeling notes

- **servicecidrs IS present in this schema.** The brief flagged ServiceCIDR as a
  possibly-beta API that might be absent; `grep -n "entity servicecidrs"
  k8s.cedarschema` confirms both `servicecidrs` and `servicecidrs_status` entities
  exist, so this transition is **runnable and synthesized [sound]**, not marked
  NOT SYNTHESIZABLE.
- **Name equality only.** Both parents are cluster-scoped (no `namespace`
  attribute), so unlike the namespaced subresource pattern (`pods_status → pods`,
  which also needs `resource.namespace == context.resource1.namespace`) the
  relation is pure name equality.
- **Verb family.** `get`/`update`/`patch` on `/status` all address the same stored
  object; the mutating verbs are the capability of interest (writing the parent's
  status), folded into the single `update` transition per resource.
- **Severity Medium.** VolumeAttachment status carries attach/detach state and
  errors a CSI attacher reports; writing it on the parent can mislead attach
  reconciliation. ServiceCIDR status reflects range readiness/finalizers;
  tampering can affect Service IP allocation. Both are integrity, not confidentiality,
  risks.
