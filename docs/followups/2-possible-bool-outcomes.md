# Follow-ups for branch 2 — possible-bool-outcomes

## PR description

Adds `Residual::possible_bool_outcomes()` to the type-aware partial evaluator: given the
residual TPE leaves behind on partially known data, it tells which of `true`, `false` and
"error" the condition can still produce once the unknowns are filled in — structurally, with no
solver, as a sound over-approximation. It is the baseline that the symbolic evaluator later
refines, and what lets a policy set record "this policy can never be true in this request
environment" without evaluating anything.

```rust
// residual of `principal.age > 18 && resource.public` with an unknown principal
let outcomes = residual.possible_bool_outcomes();
assert_eq!(outcomes, nes![True, False, Error]); // `>` on a Long may overflow-error
// residual of `context.flag` (a Bool that cannot error)
assert_eq!(residual.possible_bool_outcomes(), nes![True, False]);
```

## What this branch contains

- `Residual::possible_bool_outcomes` and `EvaluationOutcome` in
  `cedar-policy-core/src/tpe/residual.rs`, with hand tables and a brute-force reference check
  as tests; `nonempty-collections` as a dependency of `cedar-policy-core`.

## Review findings

- `possible_bool_outcomes` carries a `TODO(luxas)` about relying on the validator's singleton
  boolean types versus resolving simple cases in the TPE evaluator "by hand"; the two agree
  today, so the comment should be resolved (keep the validator refinement, drop the TODO).
- `EvaluationOutcome::Error` has a `TODO: Split into RuntimeError, EntityNotExistError and
  TypeError?`; downstream (the symbolic evaluator) only needs the three-valued form.
- The function sits behind the `tpe` feature; with branch 1's defaults it is compiled and
  tested by a plain `cargo test`, but a release build without `tpe` will not have it, which
  every consumer must gate on.
- The test module is ~1000 lines inside `residual.rs`; `tpe/residual/tests.rs` would keep the
  source file readable.

## Divergences from the private source

- None in this branch's files: `residual.rs` is the source's state before the later
  `to_typed_expr` addition and the move of `EvaluationOutcome` to `typechecked` (branch 3).

## Suggested follow-ups

- Resolve the two TODOs; consider upstreaming this function on its own, since it is independent
  of everything else in the stack.
