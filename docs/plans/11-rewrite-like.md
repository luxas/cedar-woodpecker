# Plan 11 — `like` without wildcards is `==`

## Goal

The equivalence-class rewrites of policy synthesis (branch 16) want every string atom to be
either an equality — which feeds the classes — or a genuine pattern constraint. A `like` whose
pattern has no wildcard matches exactly one string, so it is an equality in disguise:
`x like "exact"` ⇒ `x == "exact"`. This branch adds that rewrite as a standalone pass and the
predicate that says every remaining `like` is a real pattern.

## Design

- `rewrite_like(e)` walks every node bottom-up (`expr_util::with_children`, as the splitter
  does) and replaces `x like p` whose pattern contains no `PatternElem::Wildcard` by
  `x == "<the pattern's characters>"`, keeping the source location; everything else is unchanged.
  `likes_have_wildcards(e)` is the completeness predicate.
- **Sound where the operand is a string or errors**: on a non-string *value* `like` is a type
  error while `==` against a string literal is `false`, so the rewrite assumes validated input
  (the validator types `like` operands as `String`); the Lean theorem carries that hypothesis
  over the operands of wildcard-free `like`s. An escaped `\*` is a character, not a wildcard.
- Standalone: not wired into `split_atoms` or the policy pipeline (their proofs are untouched);
  the caller applies it where wanted.

## Files

- `cedar-policy-symcc/src/dnf/like.rs` (new), `src/dnf/mod.rs`, `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf`: the empty pattern (`== ""`), characters only, a
wildcard kept, an escaped star kept, nesting inside `if`/records/sets/`iferror`, the
completeness predicate before and after, solver equivalence of `e` and `rewrite_like(e)` for
well-typed `e`, and the concrete evaluator on strings with and without wildcards. The Lean
proofs and the `like-lean-drt` target are cedar-spec branch 11.

## History

Merged from the private plan "like without wildcards is == (Phase 4 Step 1)" and its revision.
