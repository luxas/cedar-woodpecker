# Plan 9 — DNF conversion of boolean structure

## Goal

Everything downstream of the symbolic evaluator that wants to reason about *how* a policy can
be true — splitting a policy into one policy per way, eliminating quantified variables for
policy synthesis — needs the policy condition in disjunctive normal form: a list of cubes, each
a conjunction of atoms or negated atoms, of which at most one is true on any input. Cedar's
`&&` and `||` are three-valued and short-circuiting, so the textbook rewrite is wrong:
`(a || b) && c` is **not** `(a && c) || (b && c)` — for `a = true, c = false` the original never
evaluates `b`, which may error, while the rewrite does. This branch adds a converter that is
exact under Cedar's evaluation semantics, generic over the expression's metadata, with the
"which atoms may error" question delegated to the caller.

## Semantics

Outcomes are `{T, F, E}`: `a && b` is `E → E`, `F → F`, `T → b`; `a || b` is `E → E`,
`T → T`, `F → b`; `!a` flips `T`/`F`; `if c then a else b` is `E → E`, `T → a`, `F → b`.
Equivalence means the same outcome on every input (the error kind is not preserved). Structure
versus atom exactly as the evaluator's tree: `And`, `Or`, `Not`, `If` are structure, a boolean
literal is a leaf, everything else is an atom — opaque, pure, may error.

## Design

- **Paths, not rewrite rules.** `paths(e)` linearises the evaluation decision tree: every
  root-to-leaf path is a cube (the atoms along it in evaluation order, negated on a `false`
  edge). Paths to a `true` leaf are the cubes that can be true; paths to a `false` leaf are
  *never-true* cubes, rendered with a trailing `&& false`, that exist only to reproduce errors.
  `!x` flips the leaves; `a && b` grafts `paths(b)` onto the true leaves of `paths(a)`; `||` is
  the dual; `if` grafts both branches in one pass over the test's paths. For `(a || b) && c`
  this gives `(a && c) || (!a && b && c)` — the guard `!a` is kept on purpose so that every cube
  describes on its own when its path is taken.
- **Why it is exact.** On any input the original follows exactly one path; a cube is `false`
  where it leaves that path, an error where it contains the erroring node, and the path's own
  leaf when it *is* the path. So at most one cube is ever true, the order of the cubes is
  irrelevant, and the disjunction errors exactly when the original does provided some kept cube
  contains the erroring node.
- **Literal dedup.** A literal whose (erased) atom already occurs earlier in the cube is
  determined by the prefix: same polarity ⇒ drop it, opposite polarity ⇒ truncate and mark
  never-true.
- **Coverage pruning.** A never-true cube is kept only if it has a node — an atom under its
  prefix — that `can_error` and that lies on no cube kept so far. `can_error` is supplied by
  the caller: `Dnf::of_expr` answers `true` for every non-literal atom (always sound); a
  caller with evaluator metadata answers `!a.data().is_error_free()`, and the DNF is then
  equivalent to the input wherever those answers hold — the evaluator's own envelope.
- **Independent of the evaluator.** `src/dnf` imports nothing from `crate::evaluator` and vice
  versa; the evaluator is a test oracle only. The shared structure-preserving rebuilds
  (`erase`, `with_data`, built with `ExprBuilder::with_expr_kind`, never the constant-folding
  `and`/`or`) live in `expr_util`.
- **Budget.** DNF is exponential; `Dnf::of` fails with `DnfError::TooLarge` past a cube budget
  (`DEFAULT_MAX_CUBES = 4096`); the recursion guard maps to `RecursionLimit`.
- **API.** `Dnf::of(expr, can_error, max_cubes)` / `Dnf::of_expr(expr)`; `Literal`, `Cube`,
  `Dnf` with `cubes()`, `true_cubes()`, `to_expr()` (an `||`-chain of `&&`-chains, right-nested
  to match the Lean model's rendering, `true`/`false` for the empty cases); `interpret`, the
  three-valued short-circuit semantics over opaque atoms — the truth-table oracle.

Rejected: textbook distribution (wrong under error semantics); guard elimination
(`a || (!a && b) → a || b` is valid but makes a cube depend on its predecessors, which the
later passes must not have); a structural can-error analysis (the evaluator gives exact sets).

## Files

- `cedar-policy-symcc/src/dnf/{mod,paths,interpret}.rs`, `src/lib.rs`, `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf` (cvc5): rule tables for the README's shapes;
an exhaustive three-valued truth-table check over every boolean structure of up to six nodes
over three atoms (`interpret(dnf) == interpret(e)` for every assignment, normal form, no
never-true cube with an all-error-free `can_error`); every table case solver-checked with
`check_equivalent`; evaluator metadata fed through the closure drops and keeps the right cubes;
the budget. The Lean model, proofs and the `dnf-lean-drt` target are cedar-spec branch 9.

## History

Merged from the private plan "DNF conversion of boolean structure (Phase 3, Step 1)" and its
revision; the rule tables' corrections found while deriving the tests are folded in.
