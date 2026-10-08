# create/update/patch RoleBinding → `bind` Role

**Paper:** Yang et al., CCS '25 §4 (RBAC privilege escalation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create rolebindings --> bind roles`
- `update rolebindings --> bind roles`
- `patch rolebindings --> bind roles`

## The implicit permission

Let *p* = "may create (or update/patch) a RoleBinding" and *q* = "may `bind` a
Role to a subject". Creating a RoleBinding is the act of binding a Role's rules to
a subject — the `bind` verb in RBAC is defined precisely as the permission to
grant a particular Role through a (Cluster)RoleBinding. So a principal who can
write RoleBindings implicitly holds the `bind` capability over the Roles those
bindings reference, letting them hand any Role's rules (up to the ones they
themselves hold, absent the escalation escape hatch) to any subject.

## Why it exists (Kubernetes mechanism)

The RoleBinding registry runs a privilege-escalation-prevention check on write.
A subject may create a binding to a Role only if they already hold the Role's
rules (`ConfirmNoEscalation`) **or** they are explicitly authorized to `bind`
that specific Role (`BindingAuthorized`). Possessing the `bind` permission is
therefore the dedicated capability that "creating a RoleBinding that references a
Role" exercises.

Source: [`rolebinding/policybased.Storage.Create`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/rbac/rolebinding/policybased/storage.go#L83) — short-circuits the escalation check when `BindingAuthorized(roleBinding.RoleRef, …)` holds, i.e. when the caller has `bind` on the referenced Role.
Source: [`BindingAuthorized`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/rbac/escalation_check.go#L101) — checks the `bind` verb on the referenced role object.
Source: [`ConfirmNoEscalation`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/rbac/validation/rule.go#L53) — the alternative gate: you may bind rules you already hold.

## Transition function

Uses derived field(s): none (constrains the target by `resource.resourceCombined`).

`rolebinding-binds-role.json`:
```json
{ "transitions": [ {
  "name": "rolebinding-binds-role",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "io::k8s::authorization::rbac::rolebindings" } ],
  "target":  { "action": "k8s::Action::\"bind\"", "resource": "k8s::Resource" },
  "when": "resource.resourceCombined == \"roles\""
} ] }
```

## Example

Explicit policy (`rolebinding-binds-role.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::authorization::rbac::rolebindings
);
```

## Synthesized implicit policy

```
// rolebinding-binds-role: (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::authorization::rbac::rolebindings) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"bind", resource: k8s::Resource) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.resourceCombined == "roles")
@woodpecker("rolebinding-binds-role: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"bind",
  resource is k8s::Resource
) when {
  resource.resourceCombined == "roles"
};
```

## Modeling notes

- **`bind` applies only to `k8s::Resource`.** Confirmed by grep: the `action
  "bind"` block's `appliesTo.resource` is `[k8s::Resource]`. There is no
  `k8s::Action::"bind"` over `io::k8s::authorization::rbac::roles` in this schema,
  so the honest target is `bind` on the catch-all `k8s::Resource`, narrowed to the
  Roles kind with `resource.resourceCombined == "roles"`. This matches the paper
  row `--> bind roles`.
- **Over-approximation.** The transition does not model *which* Role is bound or
  whether the binder already holds its rules — it captures the `bind` capability
  itself, which is the privilege the explicit grant represents. The real
  escalation is still gated by `ConfirmNoEscalation` unless the binder has `bind`
  (or `escalate`); this file formalises the capability on the `bind`-authorized
  side of that gate.
- **Severity.** Critical: `bind` is the primitive that lets a principal widen any
  subject's authority up to the rules they can bind, and is a classic lateral /
  vertical escalation step. update/patch collapse into the same mechanism (RBAC
  re-evaluates the mutated binding's `roleRef`).
