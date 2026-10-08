# update/patch/get CertificateSigningRequest/approval → update/patch/get CertificateSigningRequest

**Paper:** Yang et al., CCS '25 §4 (CSR approval → identity issuance); severity **Critical** for update/patch, **medium** for get (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update certificatesigningrequests/approval --> update certificatesigningrequests`
- `patch certificatesigningrequests/approval --> patch certificatesigningrequests`
- `get certificatesigningrequests/approval --> get certificatesigningrequests`

## The implicit permission

Let *p* = "may update the `approval` subresource of a CSR" and *q* = "may update
the parent CSR". Writing the `Approved` condition through the approval
subresource flips the CSR into an approved state; the signer controller then
issues the requested X.509 client certificate. That certificate is a *new
identity* (arbitrary CN / O ⇒ arbitrary user / groups to the API server). So
approval authority over a CSR implies control of the parent CSR object and,
through it, minting of a client identity of the requester's choosing. The runnable
transition formalises the subresource→parent edge (`approval` ⇒ parent); the
same shape holds for patch (same verb family) and, more weakly, get.

## Why it exists (Kubernetes mechanism)

`certificatesigningrequests/approval` is a dedicated REST endpoint whose update
strategy only touches the Approved/Denied conditions; it is split out from the
main CSR store precisely so that `approve` authority can be granted separately.
Approving drives the signer to produce `status.certificate`.

Source: [`ApprovalREST.Update`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/certificates/certificates/storage/storage.go#L145) — the `/approval` subresource writer, built from the shared store with the [`ApprovalStrategy`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/certificates/certificates/storage/storage.go#L70) (`approvalStore := *store; approvalStore.UpdateStrategy = csrregistry.ApprovalStrategy`).
Source: [`csrApprovalStrategy.ValidateUpdate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/certificates/certificates/strategy.go#L296) — "Updating the approval should only update the conditions", confirming this subresource is exactly the approve/deny lever on the parent CSR.

## Transition function

Uses derived field(s): none. CSRs are **cluster-scoped** (grep: the
`certificatesigningrequests` and `certificatesigningrequests_approval` entities
carry no `namespace` field), so the match is on `name` only — no namespace clause.

`csr-approval-issues-cert.json`:
```json
{ "transitions": [ {
  "name": "csr-approval-issues-cert",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "io::k8s::certificates::certificatesigningrequests_approval" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "io::k8s::certificates::certificatesigningrequests" },
  "when": "resource.name == context.resource1.name"
} ] }
```

## Example

Explicit policy (`csr-approval-issues-cert.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::certificates::certificatesigningrequests_approval
);
```

## Synthesized implicit policy

```
// csr-approval-issues-cert: (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::certificates::certificatesigningrequests_approval) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: io::k8s::certificates::certificatesigningrequests) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.name == context.resource1.name)
@woodpecker("csr-approval-issues-cert: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::certificates::certificatesigningrequests
);
```

## Modeling notes

- **One runnable file, three rows.** The `.json`/`.cedar` encode the `update`
  edge. `patch` is the identical mechanism (the patch verb hits the same
  ApprovalStrategy) and the same shape is sound by substituting
  `k8s::Action::"patch"` throughout. `get approval --> get parent` is the weaker
  read edge (severity medium) and is subsumed by the same subresource→parent
  identity; it is not separately shipped because it is the trivial read-side of the
  same mechanism.
- **Cluster-scoped match.** Unlike the namespaced subresource patterns, the `when`
  omits any `namespace` comparison because neither entity has a `namespace`
  attribute — adding one would fail to typecheck.
- **Escalation reach.** Approving CSRs lets an attacker request a cert with CN
  `system:admin` / O `system:masters` and have it signed, i.e. mint a
  cluster-admin-equivalent identity — hence Critical for the write edges.
