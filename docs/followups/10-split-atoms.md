# Follow-ups for branch 10 — split-atoms

## PR description

Adds `dnf::split_atoms`: it rewrites an expression so that no atom hides an `if`, `&&`, `||` or
`!` — such structure is hoisted out into the boolean skeleton, guarded so that the result
evaluates *exactly* like the original (same value, same error). After it, every atom of a DNF
cube is an opaque term the later rewrites (literal elimination, equivalence classes, quantifier
elimination) can work on.

```rust
let e = parse(r#"principal.age == (if principal.vip then 30 else 18)"#);
println!("{}", split_atoms(&e, DEFAULT_MAX_SPLIT_NODES)?);
// if (principal.age == principal.age)
//   then (if principal.vip then principal.age == 30 else principal.age == 18)
//   else false
```

## What this branch contains

- `src/dnf/split.rs` (`split_atoms`, the hoisting primitive, the guard machinery, `iferror`
  opacity, the node budget); `mod.rs`'s Step 2 doc and the `and_chain`/`conjuncts`/`disjuncts`
  helpers; `paths.rs` using `split::rebuild_err`; the Step 2 tests.

## Review findings

- The guard machinery (`guard`, `extend`, `learn_false`, `add_guards`, `erase_and_fold`, the
  evaluated closure) is `pub(super)` for the elimination pass (branch 12) and is the real API
  of this module; a `guards.rs` submodule with its own doc would make the contract visible.
- `split_generated_sweep` builds random expressions in-process with the solver as oracle; the
  Lean DRT covers the same space against the model — both stay.
- The exact-evaluation guarantee costs guards that a later simplification drops again; a mode
  that skips guard generation for callers that only need the decision would avoid the round trip.

## Divergences from the private source

- Comments cite the reverted opaque-equality experiment by description instead of the private
  plan numbers.
- `tests/dnf.rs`: `split_preserves_error_kind` uses the concrete-authorizer fixtures of branch
  14 and lands there.

## Suggested follow-ups

- `guards.rs`; the decision-only mode.
