# create APIService → get/watch Service

**Paper:** Yang et al., CCS '25 §implicit-permission catalogue; severity **High** (classification.txt — a cluster-scoped registration that redirects aggregated API traffic to an arbitrary backing Service).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create apiservices --> get services`
- `create apiservices --> watch services`
- `update apiservices --> get services`
- `patch apiservices --> get services`

(Registering/updating an APIService all point the aggregator at the Service it
names in `spec.service`; get and watch are the same read path the aggregator
performs on that backing Service, collected into one mechanism.)

## The implicit permission

Holding `create` on cluster-scoped `apiservices` lets the principal register an
aggregated API whose backing is **any Service in any namespace**. The
kube-aggregator then **reads and watches that Service** (to resolve its
ClusterIP / endpoints and proxy requests to it) using the aggregator's own
credentials. Explicit *p* = "create an APIService"; implicit *q* = "cause the
aggregator to get/watch an arbitrary backing Service".

## Why it exists (Kubernetes mechanism)

An `APIService` with `spec.service` set registers an aggregated API served by a
Service. The aggregator's proxy handler reads `apiService.Spec.Service.{Name,Namespace,Port}`
and resolves the endpoint via a `ServiceResolver`, which looks the Service up in
a shared lister (a get) and watches it for changes. The backing Service is named
purely in the APIService spec and may live in any namespace.

Source: [`proxyHandler ... apiService.Spec.Service`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/kube-aggregator/pkg/apiserver/handler_proxy.go#L216) — reads the backing Service name/namespace/port from the APIService spec.
Source: [`aggregatorEndpointRouting.ResolveEndpoint`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/kube-aggregator/pkg/apiserver/resolvers.go#L45) — resolves the backing Service (via the Service lister) to a URL the aggregator proxies to.

## Transition function

Uses derived field(s): none. The schema's `apiservices` entity carries no
derived `serviceRef`/`serviceNamespace` field, so the backing Service cannot be
related by name. The transition is modelled with `when "true"` — an intentional
over-approximation (see Modeling notes).

`apiservice-reaches-services.json`:
```json
{ "transitions": [ {
  "name": "apiservice-reaches-services",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "io::k8s::apiregistration::apiservices" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::services" },
  "when": "true"
} ] }
```

## Example

Explicit policy (`apiservice-reaches-services.cedar`):
```cedar
// A user who may register aggregated APIs (cluster-scoped APIService objects).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::apiregistration::apiservices
);
```

## Synthesized implicit policy

```
// apiservice-reaches-services: (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::apiregistration::apiservices) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::services) [sound]
// path: context.action1 == k8s::Action::"create"
@woodpecker("apiservice-reaches-services: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::services
);
```

## Modeling notes

- **Over-approximation (whole-cluster read, no serviceRef).** The true relation
  is `target service == apiService.spec.service` (one Service, named in the
  spec, in any namespace). The schema gives `apiservices` no derived back-reference
  to that Service, so `when "true"` is the soundest expressible fact: it permits
  the aggregator to get *any* Service. This is correct as an upper bound (the real
  reachable set is a subset) but is deliberately broad. A future tightening would
  add a derived `io::k8s::apiregistration::apiservices.{serviceName,serviceNamespace}`
  pair and then `when "resource.name == context.resource1.serviceName && resource.namespace.name == context.resource1.serviceNamespace"`.
- **Cluster-scoped source.** `apiservices` have no `namespace`; the backing
  Service is namespaced, which is why no namespace equality can be asserted here.
- **Severity High.** An APIService can override an aggregated API group/version
  and redirect its traffic to a Service the attacker controls, enabling
  man-in-the-middle of API calls to that group; the get/watch on the backing
  Service is the read half of that capability.
