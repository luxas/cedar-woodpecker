# Follow-ups for branch 5 — symbolic-evaluator

## PR description

Adds a third evaluator to `cedar-policy-symcc`: given assumptions about the input — an
expression that must hold, a policy set's decision, complete or partial entity data and
requests — `Evaluator::evaluate` returns a boolean expression with every visited node annotated
with the **exact** set of outcomes it can still produce (`True`, `False`, `Error`, decided by the
SMT solver rather than by structure) and the determined nodes folded away. Where TPE says
"unknown", this says "never true under these facts". It is the engine behind the policy
simplifier, the DNF pipeline and policy synthesis in the following branches.

```rust
let mut ev = Evaluator::new(CedarSymCompiler::new(LocalSolver::cvc5()?)?, &schema)?;
ev.assume_expr(parse("principal.age > 18"));
let r = ev.evaluate(&parse("principal.age > 10 && resource.public"), &env, []).await?;
// r == `resource.public`, root outcomes {True, False}: the left operand folded to `true`
let r = ev.evaluate(&parse("principal.age < 10"), &env, []).await?;
// r == `false`
```

## What this branch contains

- `cedar-policy-symcc/src/evaluator/`: the evaluation tree, trail, outcome questions, entity
  existence via `exists[E]`, phantom values, kept guards, `evaluate` / `evaluate_observed` /
  `check_equivalent` over `Typecheckable`, the recorded `EvaluationTrace`; `assume.rs`;
  `SymbolicFold` for `TypecheckedPolicySet` (`src/typechecked.rs`); `expr_util.rs`; the pieces
  SymCC exposes for it; hand-derived tables in `tests/`; small core additions (`get_as_bool`,
  `stack_size_check` public, `Response::request/schema`, `TpeResponse::request_env/schema`).

## Review findings

- `cedar-policy-symcc/src/lib.rs`: `pub mod typechecked; // TODO(luxas): Selective exports` —
  the module's whole surface is public; decide the API (probably `SymbolicFold` only) and drop
  the TODO.
- `evaluator/mod.rs` is ~1400 lines with the tree, the questions, existence, phantom values and
  kept guards in one file; `existence` and `kept` could be their own modules like `assume`.
- `//!` comments in `evaluator/mod.rs` refer to a `README.md` in the directory that this
  branch does not carry (the design narrative is in `docs/plans/5-symbolic-evaluator.md`).
- The evaluator typechecks a bare `Expr` through the wrapper policy on every `evaluate`; with
  branch 3 in place callers should be nudged to `TypecheckedPolicySet` (the doc says so, the
  API does not).
- Three evaluators (this one, TPE, concrete) each carry an `iferror` special case and their own
  "possible outcomes" notion; a shared definition would remove drift.

## Divergences from the private source

- `cedar-policy-core/src/evaluator.rs`: `stack_size_check` gained a doc comment (CI's
  `-D warnings` with `missing_docs` rejects an undocumented `pub fn`).
- `cedar-policy-core/src/tpe/response.rs`: an unused import (`types::RequestEnv`) is not carried.
- `cedar-policy/src/api/tpe.rs`: the `use crate::{…}` block is rustfmt-formatted.
- `cedar-policy-symcc/Cargo.toml`: no `serde`/`serde_json` yet (branch 7), no `tokio-util`
  (scratch only); `symcc/solver.rs` and `solver_pool.rs` are unchanged from upstream (the
  `PipeMap` generics belong to the scratch binaries).
- Code comments cite plans by their new numbers (`plan 5` for the kept-guard rule, `plan 3` for
  the typechecked set).
- The evaluator `README.md` is not part of this branch.

## Suggested follow-ups

- Fix the exports; split `mod.rs`; a shared outcome definition.
