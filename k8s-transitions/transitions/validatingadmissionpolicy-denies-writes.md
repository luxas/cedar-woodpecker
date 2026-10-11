# create/update/patch ValidatingAdmissionPolicy → DENY any write (and delete VAP → re-allow)

> **NOT SYNTHESIZABLE (deny-side).** A VAP's CEL expression *rejects* admitted
> writes; it is not a permit a principal gains. Even the inverse `delete VAP -->
> allow …` rows are the *removal of a deny*, not a new permit for woodpecker to
> synthesize. `cedar-woodpecker` emits only PERMIT policies, so no transition is
> produced. No `.json`/`.cedar` ships and `verify.sh` skips it.

**Paper:** Yang et al., CCS '25; severity **High** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`), 20 rows (the
status-subresource rows are a separate subresource→parent mechanism):

Write-a-VAP installs a deny:
- `create validatingadmissionpolicies --> deny create [any kubernetes api resource]`
- `create validatingadmissionpolicies --> deny update [any kubernetes api resource]`
- `create validatingadmissionpolicies --> deny patch [any kubernetes api resource]`
- `create validatingadmissionpolicies --> deny delete [any kubernetes api resource]`
- `update validatingadmissionpolicies --> deny create [any kubernetes api resource]`
- `update validatingadmissionpolicies --> deny update [any kubernetes api resource]`
- `update validatingadmissionpolicies --> deny patch [any kubernetes api resource]`
- `update validatingadmissionpolicies --> deny delete [any kubernetes api resource]`
- `patch validatingadmissionpolicies --> deny create [any kubernetes api resource]`
- `patch validatingadmissionpolicies --> deny update [any kubernetes api resource]`
- `patch validatingadmissionpolicies --> deny patch [any kubernetes api resource]`
- `patch validatingadmissionpolicies --> deny delete [any kubernetes api resource]`

Delete-a-VAP removes a deny (re-allow):
- `delete validatingadmissionpolicies --> allow create [any kubernetes api resource]`
- `delete validatingadmissionpolicies --> allow update [any kubernetes api resource]`
- `delete validatingadmissionpolicies --> allow patch [any kubernetes api resource]`
- `delete validatingadmissionpolicies --> allow delete [any kubernetes api resource]`
- `deletecollection validatingadmissionpolicies --> allow create [any kubernetes api resource]`
- `deletecollection validatingadmissionpolicies --> allow update [any kubernetes api resource]`
- `deletecollection validatingadmissionpolicies --> allow patch [any kubernetes api resource]`
- `deletecollection validatingadmissionpolicies --> allow delete [any kubernetes api resource]`

(The sibling `get/update/patch validatingadmissionpolicies/status -->
get/update/patch validatingadmissionpolicies` rows are a subresource→parent
*permit* mechanism and belong to a different catalogue entry.)

## The implicit permission

A principal who may `create`/`update`/`patch` a `ValidatingAdmissionPolicy` (plus
a bound `ValidatingAdmissionPolicyBinding`) can author a CEL `validations`
expression that evaluates to `false` for a chosen set of requests. Any matching
`create`/`update`/`patch`/`delete` on *any* API resource is then **rejected** at
admission. Conversely, a principal who may `delete`/`deletecollection` such a VAP
can tear down an existing policy and thereby *re-allow* the writes it was
blocking.

State plainly: p = `create|update|patch validatingadmissionpolicies`;
q = the *inability* of principals to perform matching writes (a deny). The
`delete VAP` rows give p = `delete|deletecollection validatingadmissionpolicies`;
q = removal of an existing deny — restoring the baseline, not minting a new
permit.

## Why it exists (Kubernetes mechanism)

ValidatingAdmissionPolicy is in-process CEL admission (GA in 1.30). On each
mutating request the apiserver's policy plugin evaluates every bound policy's
CEL `validations`; a failing expression with `validationAction: Deny` returns a
rejection, blocking the write. Nothing in this path *grants* a capability — it
only subtracts.

Source: [`validator.Validate` sets `decision.Action = ActionDeny`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/validator.go#L168)
— when a CEL validation evaluates false, the decision becomes a deny.

Source: [`dispatcher` `ActionDeny` → `deniedDecisions`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/dispatcher.go#L234)
— the dispatcher collects denied decisions and rejects the admitted request. A
`delete` of the VAP simply removes the policy from this evaluation set.

## Why not a woodpecker transition

1. **Deny polarity.** The forward rows produce a `forbid`, not a `permit`.
   woodpecker synthesizes only `permit`; it has no `forbid` target to emit, and
   the writer gains no `(action, resource)` right.
2. **"Any resource" target.** The deny applies to an unbounded, dynamically-CEL
   -selected set of `(action, resource)` pairs. There is no single concrete
   target entity to put in a transition `target`, so even a hypothetical
   deny-synthesizer has no finite signature to escalate.
3. **The `delete VAP --> allow …` inverse is still not a permit.** Removing a
   deny restores whatever permits already existed; it does not *create* a permit.
   woodpecker's soundness contract is "sources ⇒ an extra permit"; "sources ⇒ a
   previously-blocked permit is no longer blocked" is not expressible as an
   additive permit transition and is unsound to model as one (it would claim the
   deleter is granted the write, which is false — the write was always granted;
   it was merely intercepted).

No transition JSON/Cedar is emitted; the entry is deny-side.

## Modeling notes

- Severity **High**: a VAP-writer can impose a cluster-wide, resource-agnostic
  freeze (or, via delete, dismantle a protective policy); the blast radius is
  "any kubernetes api resource," which is why the paper lists it per-verb.
- A full deny-side catalogue could model this as a parameterized `forbid` whose
  scope is the CEL match constraints, but Cedar cannot represent arbitrary CEL
  and woodpecker does not synthesize `forbid`, so it is deliberately out of
  scope here.
