# Follow-ups for branch 12 — eliminate-aggregates

## PR description

Adds `dnf::eliminate_aggregates` and `dnf::normalize_atoms`: record literals under `.attr`,
`has`, `==` and set literals under `contains`, `containsAll`, `containsAny`, `isEmpty`, `in`
are rewritten into operations on their elements, exactly (guards reproduce the original's
errors), on a well-typed input that the entry points check. Together with the split this
leaves every atom of a cube as a plain term over attributes, tags and variables — the input the
equivalence-class and quantifier-elimination rewrites of policy synthesis need.

```rust
let e = parse(r#"[1, 2].contains(principal.age) && {x: principal.name}.x == "al""#);
println!("{}", normalize_atoms(&e, &schema, &env, DEFAULT_MAX_SPLIT_NODES)?);
// (1 == principal.age || 2 == principal.age) && principal.name == "al"
```

## What this branch contains

- `src/dnf/elim.rs` (the rules, the guard bookkeeping, the elimination budget, the typing
  check), `mod.rs` (doc, `NotWellTyped`/`RequestEnvNotFound`/`Typecheck`, `or_chain`),
  `evaluator/mod.rs` (`typecheck_expr`), the elimination table in `tests/dnf.rs`.

## Review findings

- `elim.rs` reaches into `split.rs`'s `pub(super)` guard machinery; the two passes share a
  "rewrite under guards" core that could be one module (see branch 10).
- Typing is enforced by a full typecheck per call although the caller usually has a
  `TypecheckedPolicy` already (branch 3); accepting a `Typecheckable` would remove the
  duplicate typecheck.
- The `contains`-family rules duplicate an operand per element; the budget charges the tree
  size — worth a benchmark on realistic set literals.
- `cargo doc` warns that `eliminate_aggregates`'s documentation links to the private item
  `drop_repeated` (present on the source too); make it visible or drop the link.
- The module doc's `[`split_policy`]` link is a forward reference until branch 14.

## Divergences from the private source

- `mod.rs` carries the elimination doc and error variants but not yet the Step 3/4 modules; the
  source's missing blank `//!` line between the Step 3 and elimination docs appears when branch
  14 inserts the Step 3 text.
- `tests/dnf.rs`: `elim_preserves_error_kind` and the two `split_policy` entry-point tests
  filed in the elimination section land with branch 14 (they use its fixtures and `split_policy`).

## Suggested follow-ups

- Accept `Typecheckable`; share the guard core.
