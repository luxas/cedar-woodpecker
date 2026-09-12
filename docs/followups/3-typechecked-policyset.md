# Follow-ups for branch 3 — typechecked-policyset

## PR description

Adds `PolicySet::typecheck(&schema)`: a policy set validated and typechecked **once** against a
schema, holding for every policy and every request environment the typed condition (slots
substituted), the outcomes it can still produce, and — with `tpe` — the partial-evaluation
residual that `constant_fold()` reduces against the maximally unknown request of each
environment. Tools that evaluate the same policies many times (the symbolic evaluator, the
policy simplifier, policy synthesis in later branches) read from this cache through the
`Typecheckable` trait instead of re-running the typechecker on every call.

```rust
let tps = policies.typecheck(&schema)?.constant_fold();
let p = tps.get(&PolicyId::new("policy0")).unwrap();
for env in p.request_envs() {
    println!("{env:?}: outcomes {:?}", p.outcomes(&env).unwrap());
}
// User/view/Doc: outcomes {True, False}
// User/edit/Doc: outcomes {False}        <- the action constraint folded away
```

## What this branch contains

- `cedar-policy-core/src/typechecked.rs`: `TypecheckedPolicy` / `TypecheckedPolicySet`, the
  `Typecheckable` trait with `typecheck_expr` / `typecheck_policy` as its default,
  `EvaluationOutcome` (moved from `tpe::residual`, re-exported there), `StaticError`,
  `substitute_slots`, `constant_fold`.
- `Residual::to_typed_expr` (`tpe/residual.rs`).
- `cedar-policy/src/api/typechecked.rs`: the public newtypes and `PolicySet::typecheck`.

## Review findings

- **Public API without a changelog entry**: `PolicySet::typecheck`, the newtypes,
  `Typecheckable`, `EvaluationOutcome` and `TypecheckError` are new public items of
  `cedar-policy` and `cedar-policy-core`; upstream expects a `CHANGELOG.md` line for each.
- `TypecheckedPolicy::request_types()` iterates a `HashMap` (nondeterministic order; the tests
  sort). A `BTreeMap` or the schema's own order would make per-environment output stable.
- `same_schema` falls back to a structural `ValidatorSchema` comparison on every call; sharing
  one `Arc<ValidatorSchema>` between the set and its consumers would remove it.
- The `Typecheckable` default wraps a bare expression in a permit policy to typecheck it (core
  has no public expression-level typechecker); exposing one would delete the wrapper and the
  `true && x` unwrapping consumers do.
- TPE and SymCC still run their own typechecks (recorded as not done).

## Divergences from the private source

- None in content.

## Suggested follow-ups

- Changelog entries; a `BTreeMap` for the environments; migrate TPE and SymCC onto
  `Typecheckable`.
