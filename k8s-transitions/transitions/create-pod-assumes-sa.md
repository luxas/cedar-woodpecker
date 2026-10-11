# create Pod (as a ServiceAccount) → act as the ServiceAccount

**Paper:** Yang et al., CCS '25 §4 (a Pod runs as its ServiceAccount); DataDog
**KubeHound** `POD_CREATE` / `IDENTITY_ASSUME`. Severity **Critical** — the
acquired capability is the entire permission set of the Pod's ServiceAccount.

## The implicit permission (identity assumption)

Let *p* = `create pods` with `spec.serviceAccountName = S`, and *q* = any
`(action, resource)` the ServiceAccount `S` is authorized for. The kubelet
projects `S`'s bound token into the Pod, so code in the Pod can replay it and act
as `S` — inheriting every permission `S` holds. An identity-assumption edge,
modeled by lowering.

This is the identity-assumption *generalization* of
[`pod-create-mints-sa-token.md`](pod-create-mints-sa-token.md) and
[`serviceaccount-token-is-sa-credential.md`](serviceaccount-token-is-sa-credential.md),
which model the token *mint*; this file models the full capability inheritance it
enables.

## Why it exists (Kubernetes mechanism)

`create pods` with a chosen `serviceAccountName` causes the kubelet to call the
TokenRequest API and project a bound token for that SA into the Pod. The SA the
Pod runs as is `spec.ServiceAccountName` (the `core::pods.serviceAccountRef`
derived field).

Source: [`Graph.AddPod` SA edge](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L385) — the pod→serviceAccount binding (`serviceAccountRef`).

## Transition (impersonation definition)

`create-pod-assumes-sa.imp.json`:
```json
{ "impersonations": [ {
  "name": "create-pod-assumes-sa",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "assume": "k8s::ServiceAccount",
  "when": "context.assumed.name == context.resource1.serviceAccountRef && context.assumed.namespace == context.resource1.namespace"
} ] }
```
Lowering binds the assumed SA to the Pod's `serviceAccountRef` / namespace and
cross-products with each env the SA holds, producing
`k8s::User create pods ⟹ k8s::User <action> <resource>`.

## Example

`create-pod-assumes-sa.cedar`: Alice may `create pods`, and a ServiceAccount may
`get configmaps`.

## Synthesized implicit policy

```
// create-pod-assumes-sa@get:configmaps: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0; (principal: k8s::ServiceAccount, action: k8s::Action::"get", resource: core::configmaps) via policy1.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::configmaps) [sound]
@woodpecker("create-pod-assumes-sa@get:configmaps: policy0.cube0, policy1.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::configmaps
);
```

## Modeling notes

- Bound by `core::pods.serviceAccountRef` and namespace (both typecheck directly —
  pod and SA namespaces are `k8s::Namespace`).
- Single-hop: inherits the SA's *explicit* permissions. The SA's own *implicit*
  permissions (e.g. the SA itself creating a pod to read a secret) would need a
  fixpoint — out of scope (plan 18).
- Severity Critical: `create pods` is widely granted, and a privileged default or
  named SA makes this the most common real-world escalation.
