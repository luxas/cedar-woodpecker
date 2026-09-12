# Follow-ups for branch 14 — split-policies

## PR description

Adds `dnf::split_policy` / `split_policy_set`: every policy of a set becomes one policy per
way it can be true — the true-cubes of the DNF of its (normalized) condition — with the same
effect, scope and annotations, so that each way can be analysed, renamed or folded on its own.
The authorization decision is unchanged on every input; only the diagnostics (which policies
errored) may differ.

```rust
let split = split_policy_set(&policies, &schema, DEFAULT_MAX_SPLIT_NODES, DEFAULT_MAX_CUBES)?;
// permit(principal, action, resource) when { a || b };          becomes
// @id("p.cube0") permit(principal, action, resource) when { a };
// @id("p.cube1") permit(principal, action, resource) when { !a && b };
```

## What this branch contains

- `src/dnf/policy.rs`, the Step 3 doc and re-exports in `mod.rs`, the Step 3 tests (and the
  concrete-authorizer error-kind tests of the split and elimination passes, which use these
  fixtures).

## Review findings

- `split_policy` runs the whole normalization pipeline, so this branch depends on the
  elimination (branch 12) although the narrative presents policy splitting as "Step 3"; the
  branch order reflects the code's dependencies.
- The decision theorem does not claim the `determiningPolicies`/`erroringPolicies`
  diagnostics; consumers that surface diagnostics should not be pointed at split sets without
  saying so.
- Template-linked policies are split as their static instance and the template is dropped;
  keep the template id in an annotation for provenance.

## Divergences from the private source

- `tests/dnf.rs`: `policy_entry_points_validate` also exercises `allow_cubes` and lands with
  branch 15, together with the `Policy` import of this section.
- The source's missing blank `//!` line before the elimination doc in `mod.rs` returns with the
  Step 3 text (kept as on the source).

## Suggested follow-ups

- Keep template provenance; a diagnostics-aware theorem.
