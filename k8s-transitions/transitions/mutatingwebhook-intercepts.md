# create/update/patch MutatingWebhookConfiguration → intercept/mutate any admitted object

> **NOT SYNTHESIZABLE (deny-side / interception).** A mutating webhook
> *intercepts* and rewrites objects on admission; it is not a Cedar permit over a
> concrete `(action, resource)`. `cedar-woodpecker` synthesizes only PERMIT
> policies over the schema's finite signature, and "mutate any resource on
> admission" is not such a target. No `.json`/`.cedar` ships and `verify.sh`
> skips it.

**Paper:** Yang et al., CCS '25 (Gmp Operator case study); severity **Critical**
(classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`), 3 rows:

- `create mutatingwebhookconfigurations --> create admissionreviews`
- `update mutatingwebhookconfigurations --> create admissionreviews`
- `patch mutatingwebhookconfigurations --> create admissionreviews`

Paper case study (§ Potential risks of Google apps): the **Gmp Operator**
"has the patch verb for MutatingWebhookConfigurations, and carries implicit
permissions to monitor and modify other resources. A malicious user can modify
any existing MutatingWebhookConfigurations in the GKE cluster." (paper2.txt
L1351–L1355.)

## The implicit permission

A principal who may `create`/`update`/`patch` a `MutatingWebhookConfiguration`
can register an endpoint that the apiserver calls (as an `AdmissionReview`) for a
matched set of operations. The webhook sees the full object being admitted and
may return a JSON patch that *rewrites* it — inject a sidecar, swap an image,
change a ServiceAccount, add a volume mount of a Secret. So webhook-config write
implicitly confers "observe and modify (nearly) any object as it is created or
updated," without any explicit permit on those objects.

State plainly: p = `create|update|patch mutatingwebhookconfigurations`;
q = interception — the apiserver generates an `AdmissionReview` to the webhook
and applies the webhook's mutation to the in-flight object.

## Why it exists (Kubernetes mechanism)

Mutating admission webhooks are a core extension point: the apiserver's mutating
webhook plugin dispatches each matched request to the configured webhook as an
`AdmissionReview`, then applies the returned JSON patch to the object before it
is persisted.

Source: [`Plugin.Admit`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/mutating/plugin.go#L91)
— the mutating admission entrypoint invoked for every matched mutating request.

Source: [`mutatingDispatcher.Dispatch`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/mutating/dispatcher.go#L105)
— loops the configured webhooks, sends each the `AdmissionReview`, and applies
the returned patch to the admitted object (tracked by the `mutated` annotation).

## Why not a woodpecker transition

A woodpecker transition's `target` is one concrete `(action, resource)` over the
schema (e.g. `get`/`core::secrets`), and the synthesized output is a `permit`
with a soundness proof. The mutating-webhook effect does not fit:

1. **Interception, not a request the principal makes.** The webhook does not call
   the API as the configuring principal; the *apiserver* calls the webhook and
   applies its mutation. There is no principal→(action,resource) permit to
   escalate to — the capability is "edit whatever is being admitted," which is
   not a request Cedar authorizes.
2. **"Any resource" + "mutation-on-admission."** A faithful target would need to
   be *every* resource AND carry the semantics "modify the object as it is
   created/updated." The schema has no entity or action expressing
   mutation-on-admission, so the target is outside the transition signature.
3. This is the row closest to a *real* escalation in Family G — the webhook truly
   does gain read/modify power over admitted objects — but in this permit-escalation
   model it is still modeled as interception, not a Cedar permit, so it is left
   unsynthesized rather than modeled unsoundly.

**Optional approximation (deliberately NOT shipped).** One could crudely
over-approximate the "modify" half as a permit transition
`source create|update|patch io::k8s::admissionregistration::mutatingwebhookconfigurations`
→ `target k8s::Action::"patch" / k8s::Resource` with `when: "true"`, reading
"webhook-config write ⇒ ability to patch any resource." This is left out on
purpose: it conflates admission-time interception with a direct `patch` request,
drops the "only objects currently being admitted" restriction, and would mislead
a reader into treating it as a validated escalation. It is noted here only to
show the gap, not to be synthesized.

## Modeling notes

- Severity **Critical**: the paper's Gmp Operator example shows a single `patch`
  on a webhook config escalating to monitor/modify arbitrary cluster objects — a
  cluster-takeover primitive.
- The three rows (create/update/patch) are one mechanism (register/alter the
  webhook config); the `--> create admissionreviews` RHS is the apiserver's
  call-out, i.e. the interception itself, not a request the principal issues.
- Because the real power is genuine (unlike pure deny-side quota/VAP), a future
  model extension that adds a mutation-on-admission action over `k8s::Resource`
  could capture it; that extension is out of scope for the current schema.
