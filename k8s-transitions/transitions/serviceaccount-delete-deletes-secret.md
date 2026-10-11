# delete/deletecollection ServiceAccount → delete Secret

**Paper:** Yang et al., CCS '25 §4 (controller cascade / cleanup side effects); severity **low** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `delete serviceaccounts --> delete secrets`
- `deletecollection serviceaccounts --> delete secrets`

## The implicit permission

Let *p* = "may delete a ServiceAccount *S*" and *q* = "may delete the Secret(s)
holding *S*'s auto-generated legacy token". When *S* is deleted, the
ServiceAccount token controller / legacy token cleaner garbage-collects the
Secrets of type `kubernetes.io/service-account-token` that belonged to *S*. So a
principal who can delete ServiceAccounts can cause those token Secrets to be
deleted without ever holding `delete secrets` directly.

## Why it exists (Kubernetes mechanism)

Legacy SA token Secrets are owned by their ServiceAccount; the cleanup controller
reconciles them away when the SA (or the binding reference) disappears, deleting
the Secret object via the core client.

Source: [`LegacySATokenCleaner.evaluateSATokens`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/serviceaccount/legacy_serviceaccount_token_cleaner.go#L113) — the reconcile loop over SA-token Secrets.
Source: [the Secret delete call](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/serviceaccount/legacy_serviceaccount_token_cleaner.go#L208) — `client.CoreV1().Secrets(ns).Delete(...)` for the orphaned/owned token Secret.
See also the [tokens controller](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/serviceaccount/tokens_controller.go#L344) (`deleteTokens`), the counterpart that removes an SA's token Secrets when the SA is deleted.

## Transition function

Uses derived field(s): `core::serviceaccounts.secretRefs` (see SCHEMA.md) — the
set of Secret names holding this SA's legacy tokens.

`serviceaccount-delete-deletes-secret.json`:
```json
{ "transitions": [ {
  "name": "serviceaccount-delete-deletes-secret",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"delete\"", "resource": "core::serviceaccounts" } ],
  "target":  { "action": "k8s::Action::\"delete\"", "resource": "core::secrets" },
  "when": "resource.namespace == context.resource1.namespace && context.resource1.secretRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`serviceaccount-delete-deletes-secret.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::serviceaccounts
);
```

## Synthesized implicit policy

```
// serviceaccount-delete-deletes-secret: (principal: k8s::User, action: k8s::Action::"delete", resource: core::serviceaccounts) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"delete") && (resource.namespace == context.resource1.namespace) && context.resource1.secretRefs.contains(resource.name)
@woodpecker("serviceaccount-delete-deletes-secret: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::secrets
);
```

## Modeling notes

- **Scoped by `secretRefs`.** The transition only implies deletion of the Secrets
  the SA actually owns (its legacy token Secrets), via the derived
  `serviceaccounts.secretRefs` set and a same-namespace guard — not arbitrary
  Secrets. This keeps the over-approximation tight.
- **deletecollection.** `deletecollection serviceaccounts` is the bulk form of the
  same cascade (every matched SA triggers its own cleanup), so it collapses into
  this one mechanism file.
- **Severity.** Low: this is a destructive side effect (availability of the token
  Secret), not a confidentiality/privilege gain — the attacker is deleting a
  credential, not reading one. The paper buckets both rows as low accordingly.
- **Modern caveat.** With the shift to the TokenRequest API, SAs no longer get
  auto-generated token Secrets by default; `secretRefs` is empty for such SAs and
  the implication vacuously does not fire. The mechanism remains real for legacy
  / explicitly-created token Secrets.
