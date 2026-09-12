# Plan 15 — Combining allow and deny policies into allow-only cubes

## Goal

Policy synthesis wants to reason about a policy set as a list of *allow-only* cubes: each one
a complete, self-contained condition under which a request is allowed. A forbid stands in the
way — it blocks permits from outside — so every forbid's condition must move *into* the permits
it can block. Moving a term across is only sound if a forbid that *errors* (and is therefore
ignored by the authorizer, so the permit still applies) becomes `false` rather than an error
there — which is what `iferror` (branch 4) exists for.

## Design

- **The witness.** A decision is `allow` exactly when some permit is `true` and no forbid is;
  a forbid `F = d₁ && … && dₖ` is not `true` exactly when its first non-true conjunct decides:
  `!iferror(d₁, false) || (d₁ && (!iferror(d₂, false) || (d₂ && … !iferror(dₖ, false))))`
  (`deny_witness`, nested right so the Lean model is a one-line recursion and the DRT compares
  structurally; equivalent to the flat expansion). `!iferror(d, false)` is `true` exactly when
  `d` is not `true` and never errs for boolean-or-error `d`; cube `l` is evaluated only after
  `d₁ … dₗ₋₁` are known `true`, so the witness never errs and is `true` exactly when the forbid
  is not. On a non-boolean conjunct both the forbid and its witness fail to be `true`, so the
  equivalence holds on everything that validates.
- **`combine_allow_deny(policies)`**: one permit per permit — same id, scope and annotations,
  `when` = the permit's condition conjoined (nested right, built from `And` nodes directly so no
  literal folding changes the shape) with the witness of every forbid's *full* condition (its
  filled scope constraints plus its `when`, flattened — so a template-linked forbid works),
  forbids sorted by id for determinism; forbids and unlinked templates dropped; a set with no
  forbids is copied; a permit without a `when` clause contributes no conjunct (an explicit
  `when { true }` is kept — the encoders distinguish them).
- **`allow_cubes(policies, schema, budgets)`** = `split_policy_set` of the combined set: the
  allow/deny cross terms, one allow policy each, `O(k^m)` of them under the budgets. It
  validates the *combined* permits — what its theorem assumes (a forbid is typed in the permit
  environments it can reach, past the scope conjuncts; an ill-typed forbid that no permit
  environment reaches goes unreported).
- `interpret` learns `iferror`'s table so the truth-table oracle can cover the witness.

## Files

- `cedar-policy-symcc/src/dnf/combine.rs` (new), `src/dnf/{mod,interpret}.rs`, `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf`: hand tables (`permit a; forbid b` →
`a && !iferror(b, false)`; a two-conjunct forbid; two forbids; an `||` inside a conjunct stays
one term; scope conjuncts of forbids in the chain; no forbids; no permits; a template-linked
forbid; id order), `allow_cubes` cube lists, `{T, F, E}` truth tables over generated conjunct
chains (the original decision = some combined condition true; cubes exclusive), solver
equivalence of original, combined and cubes, and the concrete `Authorizer` on erroring, denying
and allowing inputs; `policy_entry_points_validate`. The Lean model, `combineAllowDeny_decision`
/ `allowCubes_decision` and the `combine-lean-drt` target are cedar-spec branch 15.

## History

Merged from the private plan "combine allow and deny policies into allow-only cubes (Phase 3
Step 4, part 2)" and its four revisions — two of them live fuzz findings about the structural
shape the DRT compares (a permit without a `when`; `Expr::and`'s literal folding) and one a
review finding (a linked forbid's slots).
