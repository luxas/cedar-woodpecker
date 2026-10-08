# create Service → create Endpoints / create EndpointSlices

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **Medium** (classification.txt — a namespace-local object the principal did not ask to write).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create services --> create endpoints`
- `update services --> create endpoints`
- `patch services --> create endpoints`
- `delete services --> delete endpoints`
- `deletecollection services --> delete endpoints`
- `create services --> create endpointslices`
- `update services --> create endpointslices`
- `patch services --> create endpointslices`
- `delete services --> delete endpointslices`
- `deletecollection services --> delete endpointslices`

(The endpoints and endpointslice controllers are the one reconciliation
mechanism; the create/update/patch/delete/deletecollection verbs on the Service
all drive the same controller write to the derived Endpoints / EndpointSlices,
so they are collected into these two transition functions rather than ten files.)

## The implicit permission

Holding `create` (or any mutating verb) on `services` in a namespace lets the
principal cause the endpoint controllers to **write `Endpoints` and
`EndpointSlices` objects in that same namespace** — objects the principal was
never granted write access to. The explicit permission *p* is "create a Service
in namespace N"; the implicit permission *q* is "create/update/delete the
Endpoints and EndpointSlices named after that Service in N", performed by the
controllers on the principal's behalf.

## Why it exists (Kubernetes mechanism)

When a Service with a selector is created, the endpoints controller's
`syncService` builds an `Endpoints` object (same name/namespace as the Service)
and creates it; the endpointslice controller's reconciler creates one or more
`EndpointSlice` objects owned by the Service. Deleting the Service garbage-collects
them. The controllers run with their own service-account credentials, so the
Service author never needs direct endpoint RBAC.

Source: [`Controller.syncService`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/endpoint/endpoints_controller.go#L355) — reconciles a Service into an Endpoints object.
Source: [`Endpoints(service.Namespace).Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/endpoint/endpoints_controller.go#L527) — the controller creates the Endpoints in the Service's namespace.
Source: [`EndpointSlices(service.Namespace).Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/endpointslice/reconciler.go#L438) — the endpointslice reconciler creates slices in the Service's namespace.

## Transition function

Uses derived field(s): none. Both transitions key on namespace equality
(`resource.namespace == context.resource1.namespace`). Endpoints share the
Service's name and EndpointSlices are owned by (and labelled with) the Service,
but woodpecker over-approximates to any Endpoints/EndpointSlice in the same
namespace (see Modeling notes).

`service-create-creates-endpoints.json`:
```json
{ "transitions": [ {
  "name": "service-create-creates-endpoints",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::services" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::endpoints" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`service-create-creates-endpointslices.json`:
```json
{ "transitions": [ {
  "name": "service-create-creates-endpointslices",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::services" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "io::k8s::discovery::endpointslices" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`service-create-creates-endpoints.cedar` / `service-create-creates-endpointslices.cedar`):
```cedar
// A user who may create Services in namespace "shop".
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::services
)
when { resource.namespace.name == "shop" };
```

## Synthesized implicit policy

```
// service-create-creates-endpoints: (principal: k8s::User, action: k8s::Action::"create", resource: core::services) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::endpoints) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace.name == "shop") && (resource.namespace == context.resource1.namespace)
@woodpecker("service-create-creates-endpoints: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::endpoints
) when {
  resource.namespace.name == "shop"
};
```

```
// service-create-creates-endpointslices: (principal: k8s::User, action: k8s::Action::"create", resource: core::services) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::discovery::endpointslices) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace.name == "shop") && (resource.namespace == context.resource1.namespace)
@woodpecker("service-create-creates-endpointslices: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::discovery::endpointslices
) when {
  resource.namespace.name == "shop"
};
```

## Modeling notes

- **Over-approximation (name/ownership dropped to namespace).** In reality the
  controller writes an Endpoints object with the *same name* as the Service, and
  EndpointSlices that are *owned by* the Service (OwnerReference +
  `kubernetes.io/service-name` label). The schema's `endpoints` / `endpointslices`
  entities carry no derived back-reference to the owning Service, so the
  soundest fact woodpecker can prove is the namespace restriction. The synthesized
  permit is therefore broader than the true controller behaviour (any
  Endpoints/EndpointSlice in the namespace, not only the one named after this
  Service). Adding a derived `serviceName` field to `endpointslices` would let a
  future revision tighten `when` to `resource.serviceName == context.resource1.name`.
- **Verb family.** `update`/`patch` of the Service re-run the same reconcile and
  re-write the same objects; `delete`/`deletecollection` cascade-delete them via
  owner GC. All are the same mechanism, hence one transition per target kind.
- **Severity Medium.** The written objects are namespace-local and are the Service's
  own data-plane wiring; the risk is a principal shaping traffic routing (endpoint
  addresses) for a Service it can already create, not cross-namespace or secret access.
