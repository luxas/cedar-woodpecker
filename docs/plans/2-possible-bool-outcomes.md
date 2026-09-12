# Plan 2 — `Residual::possible_bool_outcomes`

## Goal

Cedar's type-aware partial evaluator (TPE) reduces a policy condition on partially known data
to a `Residual`. Consumers that reason about authorization outcomes — the symbolic evaluator of
the following branches, and the `TypecheckedPolicySet` that records what a condition can still
do in each request environment — need one question answered structurally, without a solver:
*which of `true`, `false` and "an error" can this residual still produce when the missing data
is filled in?*

`Residual::possible_bool_outcomes` answers it with a non-empty set over
`EvaluationOutcome::{True, False, Error}` that is an **over-approximation**: every outcome
concrete evaluation can actually produce is in the set, but the set may contain outcomes that
cannot occur. Callers therefore rely on an outcome's *absence* (it definitely cannot happen),
never on its presence. The set is the coarse, always-available bound that a later symbolic
evaluation refines to an exact one.

## Design

- **Leaves.** A concrete boolean value is its singleton. A partial leaf is judged by its type:
  the validator's singleton boolean types `Bool(True)` / `Bool(False)` give `{True}` / `{False}`,
  `Bool(AnyBool)` gives `{True, False}`, any non-boolean type gives `{Error}` (coercing the value
  to a boolean fails). `Error` is added whenever `Residual::can_error_assuming_well_formed`
  says the leaf may error — itself an over-approximation, so the result stays one.
- **Structure.** `&&`, `||`, `if` and `!` are followed exactly, per Cedar's left-to-right,
  short-circuiting, error-propagating semantics: for `l && r`, if `l` can be `False` the result
  can be `False`; if `l` can be `Error` the result can be `Error`; if `l` can be `True` the
  result can be whatever `r` can be (and symmetrically for `||`; `if` combines the test's
  outcomes with the branches'; `!` swaps `True` and `False`). Each leaf occurrence is treated
  independently, so `p.foo && !p.foo` is still `{True, False}`.
- **Assumption.** As for `can_error_assuming_well_formed`, the residual is assumed well formed —
  validated against a schema — so the leaf types are trustworthy.
- `EvaluationOutcome` is a plain three-valued enum (`Debug, PartialEq, Eq, Hash, Clone`); the
  set type is `nonempty_collections::NESet`, so an empty set is unrepresentable.

## Files

- `cedar-policy-core/src/tpe/residual.rs`: `EvaluationOutcome`, `possible_bool_outcomes`, and a
  large test module: hand tables over the boolean structure, and a brute-force reference that
  enumerates completions of a partial residual and checks every concrete outcome is predicted.
- `cedar-policy-core/Cargo.toml`: `nonempty-collections`.

## Verification

`cargo test -p cedar-policy-core --features tpe` (the module is behind `tpe`, on by default in
this stack). The reference check is the important one: it is what makes "over-approximation"
a tested property rather than a claim.

## History

Written before the symbolic evaluator as its structural baseline; the symbolic evaluator's
differential test against TPE (branch 5) checks its exact outcome sets against this bound.
