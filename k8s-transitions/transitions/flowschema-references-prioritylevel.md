# create FlowSchema → get PriorityLevelConfiguration

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **Low** (classification.txt — a read of cluster-scoped APF config the FlowSchema references).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create flowschemas --> get prioritylevelconfigurations`
- `update flowschemas --> get prioritylevelconfigurations`
- `patch flowschemas --> get prioritylevelconfigurations`

(create/update/patch of a FlowSchema all re-run APF's config digest, which
resolves the referenced PriorityLevelConfiguration — one mechanism.)

## The implicit permission

Holding `create` on cluster-scoped `flowschemas` lets the principal author a
FlowSchema that **references a PriorityLevelConfiguration by name**; the API
Priority and Fairness (APF) controller then **reads that
PriorityLevelConfiguration** to wire the schema's requests to a concurrency
pool. Explicit *p* = "create a FlowSchema"; implicit *q* = "cause APF to get the
PriorityLevelConfiguration named in `spec.priorityLevelConfiguration.name`".

## Why it exists (Kubernetes mechanism)

The APF configuration controller digests all FlowSchemas and, for each, resolves
`fs.Spec.PriorityLevelConfiguration.Name` against the set of known
PriorityLevelConfigurations to associate the schema with its concurrency-sharing
priority level. A FlowSchema whose referenced priority level does not exist is
marked dangling. The resolution is a read of the referenced PriorityLevelConfiguration.

Source: [`cfgMeal.digestFlowSchemasLocked`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/util/flowcontrol/apf_controller.go#L755) — digests FlowSchemas during config reconciliation.
Source: [`fs.Spec.PriorityLevelConfiguration.Name` lookup](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/util/flowcontrol/apf_controller.go#L766) — resolves the referenced PriorityLevelConfiguration by name.

## Transition function

Uses derived field(s): `io::k8s::apiserver::flowcontrol::flowschemas.priorityLevelName`
(Spec.PriorityLevelConfiguration.Name, see SCHEMA.md). The target is related by
`resource.name == context.resource1.priorityLevelName`.

`flowschema-references-prioritylevel.json`:
```json
{ "transitions": [ {
  "name": "flowschema-references-prioritylevel",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "io::k8s::apiserver::flowcontrol::flowschemas" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "io::k8s::apiserver::flowcontrol::prioritylevelconfigurations" },
  "when": "resource.name == context.resource1.priorityLevelName"
} ] }
```

## Example

Explicit policy (`flowschema-references-prioritylevel.cedar`):
```cedar
// A user who may create FlowSchemas (cluster-scoped APF config).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::apiserver::flowcontrol::flowschemas
);
```

## Synthesized implicit policy

```
// flowschema-references-prioritylevel: (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::apiserver::flowcontrol::flowschemas) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: io::k8s::apiserver::flowcontrol::prioritylevelconfigurations) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.name == context.resource1.priorityLevelName)
@woodpecker("flowschema-references-prioritylevel: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is io::k8s::apiserver::flowcontrol::prioritylevelconfigurations
);
```

## Modeling notes

- **Precise relation via derived field.** The schema exposes
  `flowschemas.priorityLevelName`, so the transition expresses the exact
  reference (`PriorityLevelConfiguration.name == FlowSchema.priorityLevelName`)
  rather than `true`. The synthesized permit scopes to the referenced priority
  level only.
- **Both source and target cluster-scoped.** FlowSchemas and
  PriorityLevelConfigurations are cluster-scoped APF objects; no namespace
  predicate applies.
- **Severity Low.** The implicit action is a *read* of cluster-wide fairness
  config (not a write and not secret data). The real abuse of FlowSchema
  authorship is binding a matching request set to a high-concurrency priority
  level, which is a separate write capability, not this read.
