# Follow-ups for branch 15 — combine-allow-deny

## PR description

Adds `dnf::combine_allow_deny` and `dnf::allow_cubes`: a policy set becomes permits only, each
carrying, for every forbid, a witness that the forbid is not true — built with `iferror` so
that an erroring forbid (which the authorizer ignores) becomes `false` rather than an error —
and then one allow policy per cube. The decision is unchanged; the result is the list of
self-contained "ways to be allowed" that policy synthesis (branch 16) consumes.

```rust
let cubes = allow_cubes(&policies, &schema, DEFAULT_MAX_SPLIT_NODES, DEFAULT_MAX_CUBES)?;
// permit(…) when { a }; forbid(…) when { b && c };            becomes
// @id("p.cube0") permit(…) when { a && !iferror(b, false) };
// @id("p.cube1") permit(…) when { a && b && !iferror(c, false) };
```

## What this branch contains

- `src/dnf/combine.rs`, `interpret`'s `iferror` arm, the Step 4 doc in `mod.rs`, the Step 4
  tests.

## Review findings

- `allow_cubes` produces `O(k^m)` cubes; the budgets stop the blow-up but there is no
  streaming form for consumers that can stop early.
- `allow_cubes` validates the *combined* permits, so a forbid no permit environment reaches is
  never typechecked and an ill-typed one goes unreported; a separate validation of the input set
  would catch it.
- `deny_witness` is public but only tests and policy synthesis use it; consider `pub(crate)`.
- `cargo doc` warns about two unresolved `[`split_atoms`]` links in the module docs (present on
  the source); they need a `super::` path.

## Divergences from the private source

- None: with this branch `dnf/mod.rs` and `tests/dnf.rs` reach their source shape.

## Suggested follow-ups

- Input-set validation; an iterator over cubes.
