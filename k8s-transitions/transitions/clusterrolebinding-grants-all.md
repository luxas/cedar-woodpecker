# create/update/patch ClusterRoleBinding → `*` on `*` (all verbs, all resources)

**Paper:** Yang et al., CCS '25 §4 (RBAC privilege escalation; Figure 3 / Kanister case study §5); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create clusterrolebindings --> * *`
- `update clusterrolebindings --> * *`
- `patch clusterrolebindings --> * *`

## The implicit permission

Let *p* = "may create (or update/patch) a ClusterRoleBinding" and *q* = "may
perform any verb on any resource in the cluster". A ClusterRoleBinding binds an
arbitrary ClusterRole — including the built-in `cluster-admin` — to an arbitrary
subject (the creator themselves, or a ServiceAccount they control). The moment
the binding exists, the bound subject is authorized for every `(verb, resource)`
tuple that ClusterRole grants, and `cluster-admin` grants `*/*`. So holding *p*
implies *q*: the whole cluster, never explicitly granted.

## Why it exists (Kubernetes mechanism)

Authorization in Kubernetes is additive and union-based: the RBAC authorizer
grants a request if *any* rule reachable from the subject's (Cluster)RoleBindings
allows it. There is no "these rules were self-granted" discount — a rule reached
through a ClusterRoleBinding the attacker just created counts exactly like one an
administrator created.

Source: [`RBACAuthorizer.Authorize`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/rbac/rbac.go#L78) — visits every rule bound to the user and allows the request as soon as one matches.
Source: [`RuleAllows`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/rbac/rbac.go#L191) — a rule with `*` verb and `*` resource matches any request, which is what `cluster-admin` carries.

Kubernetes itself recognises the danger: creating a ClusterRoleBinding that grants
more than the creator holds is gated by the privilege-escalation-prevention check
([`BindingAuthorized` / `ConfirmNoEscalation`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/rbac/escalation_check.go#L101)), unless the creator additionally holds the `bind`/`escalate` escape hatch. The transition models the capability a bare `create clusterrolebindings` grant represents once that gate is satisfied.

## Transition function

Uses derived field(s): none.

`clusterrolebinding-grants-all.json`:
```json
{ "transitions": [ {
  "name": "clusterrolebinding-grants-all",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "io::k8s::authorization::rbac::clusterrolebindings" } ],
  "target":  { "action": "k8s::Action::\"*\"", "resource": "k8s::Resource" },
  "when": "true"
} ] }
```

## Example

Explicit policy (`clusterrolebinding-grants-all.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::authorization::rbac::clusterrolebindings
);
```

## Synthesized implicit policy

```
// clusterrolebinding-grants-all: (principal: k8s::User, action: k8s::Action::"create", resource: io::k8s::authorization::rbac::clusterrolebindings) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"*", resource: k8s::Resource) [sound]
// path: context.action1 == k8s::Action::"create"
@woodpecker("clusterrolebinding-grants-all: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"*",
  resource is k8s::Resource
);
```

## Modeling notes

- **Honest encoding of "`* *`".** The paper's reference row is `create
  clusterrolebindings --> * *`: all verbs on all resources. woodpecker's target is
  a single `(action, resource)` pair, so the faithful encoding of "all verbs on
  all resources" is the catch-all action `k8s::Action::"*"` on the catch-all
  resource `k8s::Resource`. In this schema `k8s::Action::"*"` applies *only* to
  `k8s::Resource` (confirmed by grep: the `action "*"` block's `appliesTo.resource`
  is `[k8s::Resource]`), so this is the only well-typed way to say it — and it is
  exactly the right thing: `k8s::Resource` is the top of the resource lattice and
  `"*"` the top of the action lattice. This is precisely the
  privilege-escalation-prevention use case the `kubernetes-cedar-authorizer`
  README calls out — a Cedar policy over this schema is where you would *forbid*
  the escalation that the RBAC authorizer would otherwise allow.
- **`when: "true"`.** No attribute of the ClusterRoleBinding needs to match: any
  ClusterRoleBinding can name `cluster-admin` and any subject, so the implication
  is unconditional. The transition is therefore the strongest in the catalogue.
- **Severity.** Critical: full cluster compromise from one write verb on one
  cluster-scoped resource kind. update/patch are included because mutating an
  existing ClusterRoleBinding's `roleRef`/`subjects` is the same capability as
  creating one (RBAC re-evaluates the live object).
