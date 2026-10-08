# create/update/patch ResourceQuota → DENY create {workloads, PVCs, Secrets, ConfigMaps, …}

> **NOT SYNTHESIZABLE (deny-side).** This mechanism *removes* capability
> (admission rejects an over-quota create); it is not a permit a principal
> gains. `cedar-woodpecker` synthesizes PERMIT policies, so there is no
> transition to emit. No `.json`/`.cedar` ships for this entry and `verify.sh`
> skips it.

**Paper:** Yang et al., CCS '25; severity **High** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`) — the full
`create/update/patch resourcequotas --> deny create <object>` family, 33 rows:

- `create resourcequotas --> deny create pods`
- `create resourcequotas --> deny create deployments`
- `create resourcequotas --> deny create statefulsets`
- `create resourcequotas --> deny create daemonsets`
- `create resourcequotas --> deny create jobs`
- `create resourcequotas --> deny create cronjobs`
- `create resourcequotas --> deny create persistentvolumeclaims`
- `create resourcequotas --> deny create replicasets`
- `create resourcequotas --> deny create replicationcontrollers`
- `create resourcequotas --> deny create secrets`
- `create resourcequotas --> deny create configmaps`
- `update resourcequotas --> deny create pods`
- `update resourcequotas --> deny create deployments`
- `update resourcequotas --> deny create statefulsets`
- `update resourcequotas --> deny create daemonsets`
- `update resourcequotas --> deny create jobs`
- `update resourcequotas --> deny create cronjobs`
- `update resourcequotas --> deny create persistentvolumeclaims`
- `update resourcequotas --> deny create replicasets`
- `update resourcequotas --> deny create replicationcontrollers`
- `update resourcequotas --> deny create secrets`
- `update resourcequotas --> deny create configmaps`
- `patch resourcequotas --> deny create pods`
- `patch resourcequotas --> deny create deployments`
- `patch resourcequotas --> deny create statefulsets`
- `patch resourcequotas --> deny create daemonsets`
- `patch resourcequotas --> deny create jobs`
- `patch resourcequotas --> deny create cronjobs`
- `patch resourcequotas --> deny create persistentvolumeclaims`
- `patch resourcequotas --> deny create replicasets`
- `patch resourcequotas --> deny create replicationcontrollers`
- `patch resourcequotas --> deny create secrets`
- `patch resourcequotas --> deny create configmaps`

(The sibling rows `update/patch resourcequotas/status --> update/patch
resourcequotas` are a subresource→parent *permit* mechanism, and `delete
namespaces --> delete resourcequotas` is a namespace-cascade *permit*; neither is
deny-side and both belong to other catalogue entries.)

## The implicit permission

A principal who may `create`/`update`/`patch` a `ResourceQuota` in a namespace
can *shrink* the hard limits for that namespace. Once a quota's `hard` entry for
a resource (`pods`, `requests.storage`, `count/secrets`, `count/configmaps`, …)
is at or below current usage, the ResourceQuota admission plugin **rejects** any
further `create` of that object type in the namespace. So holding
write-on-ResourceQuota implicitly confers the power to *deny* other principals'
creates — a denial-of-service effect over the whole namespace, not a grant to
the quota writer.

State plainly: p = `create|update|patch resourcequotas` (namespace N);
q = the *inability* of anyone to `create <object>` in N once the new quota is
breaching. q is the **negation** of a capability, not a capability.

## Why it exists (Kubernetes mechanism)

ResourceQuota is enforced at admission time, not by any controller granting
rights. On every create/update the apiserver runs the ResourceQuota validating
admission plugin, which sums the namespace's usage against the quota's `hard`
limits and returns a `forbidden` error when the request would exceed them.

Source: [`QuotaAdmission.Validate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/resourcequota/admission.go#L161)
— the admission entrypoint invoked for each mutating request; it delegates to the
quota evaluator.

Source: [`quotaEvaluator` "exceeded quota" rejection](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/resourcequota/controller.go#L622)
— builds the `exceeded quota: %s, requested: %s, used: %s, limited: %s` error
that *denies* the create. There is no code path here that *permits* a request the
caller could not already make.

## Why not a woodpecker transition

A `cedar-woodpecker` transition encodes "holding permit(s) `sources` implies the
unstated permit `target`." Its output is always a synthesized **`permit`**. The
ResourceQuota effect is the opposite polarity:

1. The effect is a **deny** (`forbidden: exceeded quota`), not a permit. There is
   no `(action, resource)` the quota writer gains the right to perform; a third
   party *loses* the right to `create`.
2. Cedar's model layers an explicit `forbid` over `permit`s, but woodpecker does
   not synthesize `forbid` policies, and even if it did, this is not an
   escalation of the *writer's* privilege — it is an environmental side effect on
   *other* principals.
3. Modeling it as a permit (e.g. `permit(create resourcequotas) => forbid(create
   pods)`) would be unsound: it would claim the quota writer themselves cannot
   create pods, which is false, and it inverts the implication direction
   woodpecker reasons about.

Therefore no transition JSON/Cedar is emitted; the entry is documented for
completeness and marked deny-side.

## Modeling notes

- Severity **High** in the paper: a namespace-scoped DoS lever — a compromised
  principal with quota-write can freeze all new workloads/secrets/config in the
  namespace without touching any of those objects directly.
- The 33 rows collapse to a single mechanism: one admission plugin, parameterized
  only by which `hard` counter the attacker drives to zero. The object list is
  exactly the quota-countable kinds the static/LLM analysis triggered on.
- An over-approximate *deny-side* catalogue (future work) could record this as a
  `forbid` template keyed on `resource.namespace`, but it would live outside the
  permit-escalation soundness contract woodpecker checks, so it is deliberately
  left unsynthesized here.
