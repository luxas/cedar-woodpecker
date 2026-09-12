# Follow-ups for branch 16 — woodpecker

## PR description

Adds `cedar-woodpecker`, a crate and CLI that answers "what can this principal *effectively*
do?": given a policy set and a *transition function* — the system-specific rule by which one
permission implies another (anyone who can create pods can read the secrets in the pod's
namespace) — it combines the explicit permissions through the transition and synthesizes the
implied policy over the target request, with every mention of the intermediate requests
eliminated by equivalence classes, the set/`in` matrix and a sound over-approximation, and
checks each result with the solver.

```
$ cedar-woodpecker escalate --schema k8s.cedarschema --policies policies.cedar --transitions transitions.json
// pod-create-reads-secrets: policy0.cube0 (User create core::pods)
@woodpecker("pod-create-reads-secrets: policy0.cube0")
permit(principal is k8s::User, action == k8s::Action::"get", resource is core::secrets)
when { principal.groups.contains("engineers") && resource.namespace == "foo" };   // sound
```

## What this branch contains

- The crate (`src/{cubes,schema,transition,escalate,equiv,sets,expr}.rs`, the CLI), golden
  tests over `testdata/`, the crate README, the workspace member.

## Review findings

- **Research-grade.** The newest and least reviewed part of the stack: no Lean model of the
  rewrites (termination and equality up to errors are argued, not proved), no completeness
  check, no parallel pruning, principal-changing transitions unsupported; it builds and tests on
  every CI run as a workspace member. Consider a feature gate or a separate repository until
  the model exists.
- The schema extension edits the JSON schema form because the core `json_schema` structs have
  `cfg(extended-schema)` fields; a typed API for adding context attributes to an action would
  remove a fragile round trip.
- `expr::rename_vars` is a hand-written variable substitution modelled on core's
  `substitute_slots`; a general `substitute_vars` in core would serve both.
- The set/`in` goldens were cross-checked by hand against the matrix of branch 13; generating
  them from the theorem statements would keep them aligned.
- `cargo doc` warns that `sets`'s documentation links to the private `in_closure` and that
  `escalate` is both a function and a module (ambiguous link); both are on the source too.
- The `@woodpecker` annotation and `{pid}.cube{i}` / `woodpecker.{name}.{k}` ids are the
  provenance contract; document it in the crate README.

## Divergences from the private source

- Comments and the crate README cite the set matrix and the schema-alias design by this stack's
  plan numbers (13 and 16) instead of the private ones.

## Suggested follow-ups

- Feature-gate or separate the crate; the Lean model of the rewrites; `substitute_vars` in core.
