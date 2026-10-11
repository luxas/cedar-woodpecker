# get Secret (service-account token) → act as the ServiceAccount

**Paper:** DataDog **KubeHound** `TOKEN_BRUTEFORCE` (`get secrets`) /
`TOKEN_LIST` (`list secrets`). Severity **Critical** — the acquired capability is
the entire permission set of the assumed ServiceAccount.
Reference(s): KubeHound `TOKEN_BRUTEFORCE`, `TOKEN_LIST`.

## The implicit permission (identity assumption)

Let *p* = `get secrets` where the Secret is a
`kubernetes.io/service-account-token` of ServiceAccount `S`, and *q* = any
`(action, resource)` `S` is authorized for. The Secret's `data["token"]` **is**
`S`'s live bearer token, so a principal that reads it can replay it and act as
`S` — gaining every permission `S` holds. An identity-assumption edge, modeled by
lowering.

## Why it exists (Kubernetes mechanism)

A `kubernetes.io/service-account-token` Secret stores a token identifying a
ServiceAccount; the SA it belongs to is the required annotation
`kubernetes.io/service-account.name` (the `core::secrets.serviceAccountName`
derived field) and its type is `secrets.type` — both added for this mechanism
(see SCHEMA.md). Reading the Secret yields the token; the API server authenticates
it as `S`.

Source: [`SecretTypeServiceAccountToken`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/api/core/v1/types.go#L8467) — the token Secret type.
Source: [`ServiceAccountNameKey`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/api/core/v1/types.go#L8470) — the annotation naming the SA (the `serviceAccountName` derived field).

## Transition (impersonation definition)

`read-token-secret-assumes-sa.imp.json`:
```json
{ "impersonations": [ {
  "name": "read-token-secret-assumes-sa",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"get\"", "resource": "core::secrets" } ],
  "assume": "k8s::ServiceAccount",
  "when": "context.resource1.type == \"kubernetes.io/service-account-token\" && context.assumed.name == context.resource1.serviceAccountName && context.assumed.namespace == context.resource1.namespace"
} ] }
```
The `when` fires only for a token Secret (`type` guard) and binds the assumed SA
to the Secret's `serviceAccountName` / namespace. Lowering produces
`k8s::User get secrets ⟹ k8s::User <action> <resource>` for each env the SA holds.

## Example

`read-token-secret-assumes-sa.cedar`: Alice may `get secrets`, and a
ServiceAccount may `get configmaps`.

## Synthesized implicit policy

```
// read-token-secret-assumes-sa@get:configmaps: (principal: k8s::User, action: k8s::Action::"get", resource: core::secrets) via policy0.cube0; (principal: k8s::ServiceAccount, action: k8s::Action::"get", resource: core::configmaps) via policy1.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
@woodpecker("read-token-secret-assumes-sa@get:configmaps: policy0.cube0, policy1.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- **Depends on the `core::secrets.serviceAccountName` and `.type` derived fields**
  (added in this branch). Without them the `when` would not typecheck.
- **≥1.24 caveat.** Kubernetes ≥1.24 no longer auto-creates token Secrets, so this
  edge only fires where such a Secret exists (a manually-created token Secret, or a
  pre-1.24 legacy one). The e2e proof creates one explicitly.
- **`list secrets` is the verb sibling** (KubeHound `TOKEN_LIST`): same binding,
  reading every token Secret rather than a named one.
- Severity Critical: full inheritance of the SA's permissions via a replayable
  credential.
