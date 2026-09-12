# Plan 10 — Splitting atoms: hoisting boolean structure out of atoms

## Goal

After the DNF conversion (branch 9) every cube is a conjunction of atoms, but an atom may still
hide boolean structure: an `if` at any position (`(if c then x else y).field == "s"`), an `&&`
or `||` or `!` under an `==`, inside a set or record literal, or as an argument. The passes that
follow — literal elimination, equivalence classes, quantifier elimination — need atoms that are
opaque, indivisible terms. This branch adds `split_atoms`, which hoists such structure out of
atoms into the expression's boolean structure, preserving Cedar's evaluation **exactly**: the
same value, of any type, or the same error.

## Design

- **One hoisting primitive.** In an atom `A[B]`, let `B` be the first `&&`/`||`/`!`/`if` node
  in pre-order (the path from the atom's root to `B` is strict). `A[if c then x else y]` becomes
  `if c then A[x] else A[y]`; a boolean `B` becomes `if B then A[true] else A[false]`. The
  condition is now at structure position and is split recursively; `A[x]`, `A[y]` are strictly
  smaller atoms. Every node inside an atom is strict (all children evaluated left to right, the
  first error wins) and evaluation is pure, which is what makes the rewrite sound. `a == b` over
  boolean structure comes out as nested `if`s whose DNF is exactly `(a && b) || (!a && !b)` —
  the equality rule is derived, not hard-coded. `iferror(…)` calls are **opaque**: `iferror`
  catches its argument's error, so hoisting a node out of it would move that error outside the
  coalescing scope (`iferror(a && e, false)` is `false` where `e` errs; the hoisted `if` errs);
  structure *around* an `iferror` call is still hoisted.
- **Guards for exact error behaviour.** Hoisting reorders evaluation: `c` now runs before the
  atom's left siblings, so where both err the two sides would err differently. Every hoisted
  `if` is therefore wrapped as `if (g₁ == g₁ && …) then (if c then A[x] else A[y]) else false`
  for the left siblings `gᵢ` in evaluation order: `g == g` is `true` whenever `g` evaluates
  (value equality is total and reflexive) and reproduces `g`'s error otherwise. The `if … else
  false` shape rather than `G && …` keeps a non-boolean atom's value (no type error); the `else
  false` edge is a never-true cube the DNF prunes. Siblings that cannot err — literals,
  variables, set/record literals of such, after folding `lit == lit` — get no guard; guards
  already established by an enclosing wrapper or an earlier sibling are not repeated (the split
  carries the context of established guards); an aggregate guard (a set/record literal
  sibling) is decomposed into its can-err elements; guards are also *learned* from an `if`'s
  test and an `&&`'s left operand, which the scope has already evaluated; the evaluated closure
  of the context excludes never-erring nodes and repeated equalities.
- **`lit == lit` folds** to the boolean literal of value equality — total, never errs.
- **Boolean equalities are split like everything else** (`x == y` with an `&&`/`||`/`!`
  operand yields the cube set `(x && y) || (!x && !y)`).
- **Composition with the evaluator** stays a composition: split, then `Evaluator::evaluate`
  (dead branches fold, atoms get exact sets), then `Dnf::of` with the metadata's `can_error`.
- **Budget.** Hoisting duplicates the atom around `B`; each processed atom charges its node
  count against `DEFAULT_MAX_SPLIT_NODES` (`DnfError::TooLarge { what: "atom nodes" }`).
- Output is `Expr<()>`: an atom's metadata cannot describe its substituted variants.

## Files

- `cedar-policy-symcc/src/dnf/split.rs` (new): `split_atoms`, `hoist`, the guard machinery
  (`guard`, `extend`, `learn_false`, `add_guards`, `erase_and_fold`, the evaluated closure),
  `rebuild_err`; `src/dnf/mod.rs` (`and_chain`, `conjuncts`, `disjuncts`, the Step 2 doc);
  `src/dnf/paths.rs`; `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf`: the split table (hand-derived output shapes,
including left-sibling guards, a two-level hoist, aggregate siblings, an `iferror` sibling, a
boolean second operand of `==`), the generated sweep over abstract atoms with guard atoms
restricted to `{T, E}`, `iferror` opacity, the budget, the split→evaluate→DNF pipeline, and
solver equivalence of every row. The Lean model, the exact-equality theorem and the
`split-atoms-lean-drt` target are cedar-spec branch 10; `split_preserves_error_kind` (the
concrete `Authorizer` reporting the same error before and after) arrives with branch 14's
fixtures.

## History

Merged from the private plans "split atoms (Phase 3, Step 2)" and "exact error behaviour for
split_atoms" (the guards, their dedupe against the context, never-erring siblings), with the
aggregate-guard decomposition and learned guards from the elimination plan. An experiment that
made an equality of booleans an opaque atom was reverted before this stack: boolean equalities
are split.
