# `k8s-transitions` — Kubernetes implicit permissions as woodpecker transition functions

This folder formalizes the **implicit permissions** of

> Nanzi Yang, Xingyu Liu, Wenbo Shen, Jinku Li, Kangjie Lu.
> *Dangers Behind Access Control: Understanding and Exploiting Implicit
> Permissions in Kubernetes.* CCS '25, Taipei.
> [doi:10.1145/3719027.3765106](https://doi.org/10.1145/3719027.3765106)
> ([PDF in this folder](yang2025-dangers-behind-access-control.pdf))

as [`cedar-woodpecker`](../cedar-woodpecker/README.md) **transition functions**,
and confirms, for each one, *why* the implicit permission exists with a link to
the Kubernetes source that makes it possible.

## The idea in one paragraph

Yang et al. define an *implicit permission* as a relation `(p, q) ∈ R`: when a
principal exercises an explicitly granted permission `p` (e.g. `create Pod`),
Kubernetes' own controllers/kubelet/apiserver perform a further operation `q`
(e.g. `get Secret`) on the principal's behalf, even though `q` was never
granted. A `cedar-woodpecker` **transition function**
`(pt, [(at_i, rt_i)], at', rt', T)` is exactly an encoding of one such `(p, q)`:
the source environments `(at_i, rt_i)` are the explicit permissions `p`, the
target `(at', rt')` is the implicit permission `q`, and the Cedar condition `T`
is the system-specific rule relating them ("the Secret is in the Pod's
namespace and the Pod references it"). Given a policy set, woodpecker then
*synthesizes the implied policy* over `(at', rt')` — the implicit permission a
principal effectively holds — which is what turns the paper's catalogue into
something you can compute over real RBAC.

So this folder is the paper's reference set re-expressed in a form a solver can
check, over the real Kubernetes Cedar schema.

## Layout

- `yang2025-dangers-behind-access-control.pdf` — the paper (first commit).
- `SCHEMA.md` — the Cedar schema these transitions reason over, how it was
  generated from `kubernetes-cedar-authorizer`, and every **derived logical
  field** (e.g. `core::pods.secretRefs`) with the Kubernetes function that
  defines it.
- `k8s-base.cedarschema` — the generated schema, unedited.
- `k8s.cedarschema` — the base plus the derived fields; transitions validate
  against this.
- `transitions/` — **one markdown file per implicit permission / transition
  function**, each with: the paper reference, the mechanism, the *why* (a
  permalink into Kubernetes source), the woodpecker transition JSON, an example
  policy, and the synthesized implicit policy. Each `.md` has a sibling
  `<name>.json` (the runnable transition) and, where the mechanism is a
  privilege escalation woodpecker can synthesize, a `<name>.cedar` example.
- `transitions.json` — all runnable transitions in one file.
- `verify.sh` — regenerates every example with `cedar-woodpecker` and checks the
  synthesized policies are `sound`.
- `tools/schema_gen_offline.rs` — the offline schema-generation driver (see
  `SCHEMA.md`).

## Index

See [`transitions/README.md`](transitions/README.md) for the full table mapping
each of the paper's 328 triggered references
(`ImplicitPermission/Combined_LLM&Static_permission_analysis/assessment/trigger.txt`)
to the transition-function markdown that covers it, grouped by mechanism. One
markdown documents one *mechanism*; the verb families it subsumes (e.g.
`create`/`update`/`patch Deployment → create Pod` are one controller
reconciliation) are listed inside it.

## What woodpecker can and cannot synthesize

The transition model synthesizes **permit** policies — the positive implicit
permissions (`create Pod → get Secret`, `patch DaemonSet → get Secret`,
`create ClusterRoleBinding → *`). The paper also classifies **deny-side**
effects as risks: a `ResourceQuota` that *denies* workload creation, a
`ValidatingAdmissionPolicy` that *denies* writes, a webhook that intercepts.
Those are enforcement, not a permit a principal gains, so they do not fit the
permit-synthesis model; the markdown for each explains the mechanism and the
*why* but marks the transition `NOT SYNTHESIZABLE (deny-side)` rather than
shipping a runnable transition. This boundary is called out per file.

## Reproducing

```sh
# from the repo root
cargo build --release -p cedar-woodpecker
cd k8s-transitions && ./verify.sh
```

`cedar-woodpecker` needs `cvc5` on `PATH` (or `CVC5` set). See the lean/toolchain
notes in the repo root.
