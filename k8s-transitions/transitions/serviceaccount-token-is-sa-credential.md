# create ServiceAccount/token → get ServiceAccount

**Paper:** Yang et al., CCS '25 §4 / §4.5 (ServiceAccount token = SA identity; Figure 3 token-is-Secret observation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create serviceaccounts/token --> get serviceaccounts`

## The implicit permission

Let *p* = "may create a token for ServiceAccount *S* (the `serviceaccounts/token`
TokenRequest subresource)" and *q* = "may act with *S*'s identity / read *S*".
The TokenRequest subresource mints a signed bearer token for *S*. Presenting that
token to the API server authenticates the caller **as** *S*, with all of *S*'s
RBAC. So the capability to mint *S*'s token subsumes merely being able to `get`
*S* — it is strictly stronger (full impersonation of the SA, not just reading its
object).

## Why it exists (Kubernetes mechanism)

`POST serviceaccounts/{name}/token` is served by the TokenRequest REST handler,
which looks up the named ServiceAccount and asks the token generator to issue a
signed JWT bound to it. That JWT is the SA's live credential.

Source: [`TokenREST.Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L94) — the `serviceaccounts/token` subresource handler; it [fetches the ServiceAccount](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L114) and [generates and returns the token in `TokenRequestStatus`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L293).

The paper (§4.5) makes the companion observation that SA tokens are also *stored
as Secrets* ("Since ServiceAccount tokens are stored as Kubernetes Secrets, the
attacker … can extract the token"), which is why possessing a token — or the
ability to mint one — is treated as possessing the SA's identity.

## Transition function

Uses derived field(s): none (subresource → parent match on name + namespace).

`serviceaccount-token-is-sa-credential.json`:
```json
{ "transitions": [ {
  "name": "serviceaccount-token-is-sa-credential",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::serviceaccounts_token" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::serviceaccounts" },
  "when": "resource.name == context.resource1.name && resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`serviceaccount-token-is-sa-credential.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::serviceaccounts_token
);
```

## Synthesized implicit policy

```
// serviceaccount-token-is-sa-credential: (principal: k8s::User, action: k8s::Action::"create", resource: core::serviceaccounts_token) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::serviceaccounts) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.name == context.resource1.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("serviceaccount-token-is-sa-credential: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::serviceaccounts
);
```

## Modeling notes

- **Subresource/parent identity.** `core::serviceaccounts_token` and
  `core::serviceaccounts` share name + namespace (the token request targets the SA
  of the same name), matching the BRIEF's known-good "ServiceAccount token → get
  SA" pattern.
- **The target understates the real power.** `get serviceaccounts` is the row the
  paper's permission graph records, but minting a token is operationally
  equivalent to *being* the SA — every permission the SA holds becomes reachable.
  The transition formalises the graph edge, not the full impersonation blast
  radius; a downstream analysis should treat this edge as "acquire SA identity".
- **Severity.** Critical: if the SA is privileged (a controller SA,
  `cluster-admin`-bound, etc.), token minting is a direct path to that privilege.
