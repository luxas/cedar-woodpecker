# impersonate ServiceAccount → act as the ServiceAccount

**Paper:** DataDog **KubeHound** `IDENTITY_IMPERSONATE`; Kubernetes
[user impersonation](https://kubernetes.io/docs/reference/access-authn-authz/authentication/#user-impersonation).
Not in Yang et al. (their methodology targeted controller/kubelet side effects,
not direct RBAC verbs). Severity **Critical** — the acquired capability is the
*entire* permission set of the assumed identity.
Reference(s): KubeHound `IDENTITY_IMPERSONATE` (coverage: *not implemented* there).

## The implicit permission (identity assumption)

Let *p* = `impersonate serviceaccounts/<S>` and *q* = any `(action, resource)`
the ServiceAccount `S` is authorized for. A principal holding *p* can set the
`Impersonate-User: system:serviceaccount:<ns>:<S>` header, and the API server
authorizes the request **as `S`** — so the principal gains *every* permission
`S` has, parametric over all `(action, resource)`. This is an identity-assumption
edge (`⟹ Identity`), not a single `cube ⟹ cube` transition; it is modeled by
**lowering** (plan 18, Layer B): one synthesized transition per environment `S`
actually holds a permission in.

## Why it exists (Kubernetes mechanism)

Impersonation is a first-class authN feature: a request carrying
`Impersonate-User`/`-Group`/`-Uid`/`-Extra` headers, from a principal allowed the
`impersonate` verb on the corresponding resource (`serviceaccounts`, `users`,
`groups`, `uids`), is authorized as the impersonated identity. The impersonated
ServiceAccount is named by the request's resource (`io::k8s::authentication::serviceaccounts`,
`{name, namespace}`), bound here to the `k8s::ServiceAccount` principal.

## Transition (impersonation definition)

`impersonate-assumes-sa.imp.json` — `assume` is the identity's principal type; the
`when` binds it (as `context.assumed`) to the impersonate request's resource:
```json
{ "impersonations": [ {
  "name": "impersonate-assumes-sa",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"impersonate\"", "resource": "io::k8s::authentication::serviceaccounts" } ],
  "assume": "k8s::ServiceAccount",
  "when": "context.assumed.name == context.resource1.name && context.assumed.namespace.name == context.resource1.namespace"
} ] }
```
Lowering cross-products this with each environment the assumed SA has a source
cube in, e.g. `k8s::ServiceAccount get core::configmaps`, producing
`k8s::User impersonate ⟹ k8s::User get configmaps`.

## Example

`impersonate-assumes-sa.cedar`: Alice may `impersonate` ServiceAccounts, and a
ServiceAccount may `get configmaps` (a representative inherited permission).

## Synthesized implicit policy

```
// impersonate-assumes-sa@get:configmaps: (principal: k8s::User, action: k8s::Action::"impersonate", resource: io::k8s::authentication::serviceaccounts) via policy0.cube0; (principal: k8s::ServiceAccount, action: k8s::Action::"get", resource: core::configmaps) via policy1.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"impersonate") && (context.action2 == k8s::Action::"get") && (context.principal2.name == context.resource1.name) && (context.principal2.namespace.name == context.resource1.namespace) && (context.resource2 == resource)
@woodpecker("impersonate-assumes-sa@get:configmaps: policy0.cube0, policy1.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- **Sound by lowering, not by new trust.** The assumed SA's permission is a real
  source cube (source 2), so a path exists only when the SA actually holds it and
  the identity binding holds; `escalate`'s per-path solver check is unchanged.
- **Boundaries.** `impersonate users` is not cleanly modelable here: the schema's
  `io::k8s::authentication::users` is an attributeless entity, so the specific
  username cannot be bound (only an over-approximate "assume any User"). `impersonate
  groups` is not a principal assumption at all — there is no `Group` principal
  type; a group is an attribute (`.groups`) on a principal, so impersonating a
  group changes an attribute, not the principal. Both are documented, not modeled.
- Severity Critical: this is the broadest escalation — full inheritance of an
  identity's permissions, including a `cluster-admin`-bound SA.
