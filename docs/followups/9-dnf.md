# Follow-ups for branch 9 — dnf

## PR description

Adds `cedar_policy_symcc::dnf`: a converter from a boolean Cedar expression to disjunctive
normal form that is exact under Cedar's three-valued, short-circuiting semantics — each cube is
one self-contained way the expression can be true, at most one cube is true on any input, and
errors are reproduced. It is the first step of the pipeline that splits policies into one policy
per way they apply and eliminates variables for policy synthesis (branches 10–16).

```rust
let dnf = Dnf::of_expr(&parse("(a || b) && c"))?;
println!("{}", dnf.to_expr());
// (a && c) || (!a && b && c)          <- not `(a && c) || (b && c)`: `b` must not run when `a` held
```

## What this branch contains

- `src/dnf/{mod,paths,interpret}.rs`: `Dnf::of` / `of_expr`, `Literal`/`Cube`/`Dnf`, literal
  dedup, coverage pruning of never-true cubes, the cube budget, `interpret` (the truth-table
  oracle); `tests/dnf.rs` (rule tables, exhaustive small-shape truth tables, solver equivalence).

## Review findings

- `Dnf::of` takes `can_error: impl FnMut(&Expr<T>) -> bool`; the Lean model (cedar-spec branch 9)
  assumes a pure function of the atom. Either take `Fn` or state the purity requirement in the
  doc.
- `DnfError::TooLarge { limit, what }` carries a `what` string for the atom-node budget that only
  exists from branch 10; a typed enum would be clearer.
- `Literal`, `Cube`, `Dnf` have `Display`/`to_expr` renderings that are what the DRT compares
  against the Lean model (right-nested chains); that contract deserves a sentence on the type.
- `tests/dnf.rs` grows into one 2.5k-line file by the end of the stack; a file per step would
  keep each PR's tests reviewable.

## Divergences from the private source

- `paths.rs`: the `rebuild_err` mapping is a private helper of this file; the source keeps it in
  `split.rs`, where it moves with branch 10.
- `mod.rs`: without the later steps' module docs, modules, re-exports, error variants and
  helpers; `TooLarge`'s doc names only cubes.

## Suggested follow-ups

- A typed budget kind; the test-file split.
