# create Pod → create ServiceAccount/token

**Paper:** Yang et al., CCS '25 §4 / §4.5 (projected SA token mount; Figure 3 ❶→❷); severity **Critical** (companion of the `create serviceaccounts/token --> get serviceaccounts` row).
Reference(s) formalized (from `assessment/base.txt` — one of the paper's base references):
- `create Pod --> create serviceaccounts/token`

## The implicit permission

Let *p* = "may create a Pod referencing ServiceAccount *S*" and *q* = "may create
a token for *S* (the TokenRequest subresource)". A Pod with a projected
`serviceAccountToken` volume (the default for every Pod via the admission-injected
projected volume) causes the kubelet to call the TokenRequest API and mint a
bound token for the Pod's ServiceAccount *S*. So creating a Pod that names *S*
implicitly exercises `create serviceaccounts/token` on *S* — the Pod's mounted
token is *S*'s live credential, obtained without the creator ever holding
`create serviceaccounts/token` directly.

## Why it exists (Kubernetes mechanism)

Every Pod runs as a ServiceAccount (`spec.serviceAccountName`, defaulting to
`default`). The projected service-account-token volume source is realised by the
kubelet, which requests a token for the Pod's SA, bound to the Pod object, and
writes it into the container filesystem.

Source: [projected volume token minting](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/volume/projected/projected.go#L337) — `getServiceAccountToken(s.pod.Namespace, s.pod.Spec.ServiceAccountName, &authenticationv1.TokenRequest{...})`, i.e. a TokenRequest for the Pod's SA bound to the Pod.
Source: [`TokenREST.Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/serviceaccount/storage/token.go#L94) — the server side that mints the token the kubelet projects.

The paper's §4.5 narrative is the same edge from the node/filesystem side: a Pod
scheduled with a chosen SA automatically mounts that SA's token, so "create Pod"
yields the SA's token (which chains into the SA's full identity — see
`serviceaccount-token-is-sa-credential.md`).

## Transition function

Uses derived field(s): `core::pods.serviceAccountRef` (see SCHEMA.md) —
`Spec.ServiceAccountName`.

`pod-create-mints-sa-token.json`:
```json
{ "transitions": [ {
  "name": "pod-create-mints-sa-token",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::serviceaccounts_token" },
  "when": "resource.namespace == context.resource1.namespace && resource.name == context.resource1.serviceAccountRef"
} ] }
```

## Example

Explicit policy (`pod-create-mints-sa-token.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-create-mints-sa-token: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::serviceaccounts_token) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace) && (resource.name == context.resource1.serviceAccountRef)
@woodpecker("pod-create-mints-sa-token: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::serviceaccounts_token
);
```

## Modeling notes

- **Target identity match.** The target `serviceaccounts_token` entity shares the
  Pod's namespace and its name equals the Pod's `serviceAccountRef` — i.e. the
  token minted is for the SA the Pod runs as. The TokenRequest subresource's
  `name` is the SA name, so `resource.name == context.resource1.serviceAccountRef`
  is the right join.
- **Chains with `serviceaccount-token-is-sa-credential`.** `create Pod` ⇒
  `create serviceaccounts/token` ⇒ (that file) ⇒ the SA's identity. The two edges
  compose into "create Pod ⇒ acquire the referenced SA's privileges", which is the
  core of the paper's DaemonSet/Kanister escalation (Figure 3): choose a
  high-privilege SA in the pod template and read its mounted token.
- **Base-reference row.** This row lives in `assessment/base.txt`
  (`create Pod --> create serviceaccounts/token`), the hand-curated base set the
  paper's analysis builds on, not in `trigger.txt`; it is included here because the
  BRIEF names it explicitly and it is the entry edge for the SA-identity chain.
- **Severity.** Critical when the referenced SA is privileged; the creator picks
  the SA, so this is attacker-controlled.
