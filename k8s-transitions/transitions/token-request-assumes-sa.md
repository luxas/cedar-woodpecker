# create ServiceAccount/token → act as the ServiceAccount

**Paper:** Yang et al., CCS '25 §4 / §4.5 (a ServiceAccount token is the SA's
identity); DataDog **KubeHound** `IDENTITY_ASSUME` (owning an SA token assumes the
SA). Severity **Critical**: the acquired capability is the *entire* permission
set of the ServiceAccount.
Identity-assumption companion of
[`serviceaccount-token-is-sa-credential`](serviceaccount-token-is-sa-credential.md),
which records the paper's graph edge (`create serviceaccounts/token ⟹ get
serviceaccounts`) and notes that this target understates the real power.

## The implicit permission (identity assumption)

Let *p* = `create serviceaccounts/token` on ServiceAccount *S*, and *q* = any
`(action, resource)` that *S* is authorized for. The TokenRequest subresource
returns a signed bearer token for *S*. Presenting that token authenticates the
caller **as** *S*, so the caller gains *every* permission *S* has, for any
`(action, resource)`. This is an identity-assumption edge (`⟹ Identity`), modeled
by **lowering** (plan 18, Layer B): one synthesized transition per environment *S*
holds a permission in.

## Why it exists (Kubernetes mechanism)

1. **Minting.** `POST serviceaccounts/{name}/token` is served by
   [`TokenREST.Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L94).
   It [fetches the ServiceAccount named by the request](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L114)
   (same name, same namespace as the subresource) and
   [issues a signed JWT bound to it](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L293).
2. **Authenticating as the SA.** The ServiceAccount token authenticator
   [`jwtTokenAuthenticator.AuthenticateToken`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/serviceaccount/jwt.go#L353)
   validates the JWT and
   [returns the SA's `UserInfo`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/serviceaccount/jwt.go#L428)
   ([`ServiceAccountInfo.UserInfo`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/authentication/serviceaccount/util.go#L158):
   `system:serviceaccount:<ns>:<name>` plus the SA groups). Authorization then
   runs against that identity, so every RBAC grant of *S* applies.

## Transition (impersonation definition)

`token-request-assumes-sa.imp.json`: `assume` is the identity's principal type.
The `when` binds it (as `context.assumed`) to the SA the token request names.
`core::serviceaccounts_token.namespace` and `k8s::ServiceAccount.namespace` are
both `k8s::Namespace` entities, so they compare directly.
```json
{ "impersonations": [ {
  "name": "token-request-assumes-sa",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::serviceaccounts_token" } ],
  "assume": "k8s::ServiceAccount",
  "when": "context.assumed.name == context.resource1.name && context.assumed.namespace == context.resource1.namespace"
} ] }
```

## Example

`token-request-assumes-sa.cedar`: Alice may `create` `serviceaccounts/token`, and
a ServiceAccount may `get configmaps` (a representative inherited permission).

## Synthesized implicit policy

```
// token-request-assumes-sa@get:configmaps: (principal: k8s::User, action: k8s::Action::"create", resource: core::serviceaccounts_token) via policy0.cube0; (principal: k8s::ServiceAccount, action: k8s::Action::"get", resource: core::configmaps) via policy1.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.action2 == k8s::Action::"get") && (context.principal2.name == context.resource1.name) && (context.principal2.namespace == context.resource1.namespace) && (context.resource2 == resource)
@woodpecker("token-request-assumes-sa@get:configmaps: policy0.cube0, policy1.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- **Sound by lowering, not by new trust.** The SA's permission is a real source
  cube (source 2), so a path exists only when the SA actually holds it and the
  name/namespace binding holds. Two mutation checks confirm this: dropping the
  SA's `get configmaps` policy, or making the binding contradictory, both give
  `// no escalation paths`.
- **Relation to `serviceaccount-token-is-sa-credential`.** That transition keeps
  the paper's single-cube edge (`⟹ get serviceaccounts`). This definition models
  what the token actually grants. Both stay in the catalogue: the first maps the
  paper's `trigger.txt` row, this one carries the identity semantics.
- **Token lifetime.** TokenRequest tokens are bounded (`expirationSeconds`,
  default 1h), but the caller can mint a new one at any time, so the assumption
  persists for as long as *p* is held.
- Severity Critical: a privileged SA (a controller SA, a `cluster-admin`-bound SA)
  becomes directly reachable.
