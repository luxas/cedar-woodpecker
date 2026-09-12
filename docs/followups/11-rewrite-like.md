# Follow-ups for branch 11 — rewrite-like

## PR description

Adds `dnf::rewrite_like`: a `like` whose pattern has no wildcard matches exactly one string, so
it becomes an equality; every `like` left afterwards (`likes_have_wildcards`) is a genuine
pattern. A small pass that makes string atoms uniform for the equivalence-class rewrites of
policy synthesis.

```rust
rewrite_like(&parse(r#"principal.name like "alice" && resource.path like "docs/*""#))?
// principal.name == "alice" && resource.path like "docs/*"
```

## What this branch contains

- `src/dnf/like.rs`, its rows in `tests/dnf.rs`.

## Review findings

- The rewrite is standalone; `normalize_atoms` (branch 12) does not run it and policy synthesis
  calls it separately. Folding it into the pipeline would give every consumer the equality form.
- Soundness needs a string-typed operand (validated input); the Rust entry point does not
  check typing, unlike the elimination pass — document, or take a typed input.

## Divergences from the private source

- None.

## Suggested follow-ups

- Fold into `normalize_atoms`.
