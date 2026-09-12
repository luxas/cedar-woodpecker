# Plan 7 — The policy simplifier

## Goal

The validator reports a policy that is statically false as a whole, but not *why*, and not a
dead branch inside a live policy: in `(a && b) || (a && !a)` the second disjunct can never be
true, and nothing says so. The policy simplifier walks a policy set with the symbolic evaluator
(branch 5) and reports, per policy, the innermost expression whose outcome is determined in
**every** request environment that evaluates it: never true (false or an error), never false,
or the whole policy folded. Under the schema's `@semantics` assumptions (branch 6) it also
catches contradictions with the authorizer's conventions. It is a library function here;
`cedar lint` (branch 8) is its command-line and editor front end.

## Design

- `simplify(evaluator, &schema, policies_text, &assumptions) -> (Vec<Finding>, Evaluator)`
  (`cedar-policy-symcc/src/simplify.rs`). Every assumption is typechecked in every environment
  up front (`SimplifyError::AssumptionNotWellTyped { origin, env }`), and unsatisfiability is
  checked per environment by evaluating `true` under them (`UnsatisfiableAssumptions { env }`).
- Every static policy of the document that validates is typechecked once as a one-policy
  `TypecheckedPolicySet` (branch 3) and evaluated with `evaluate_observed` in every request
  environment it applies to, with the assumptions as per-call extras; the evaluator reports, for
  the root and every `&&`/`||`/`!`/`if` node it visits, the outcome set *under the trail* — the
  per-environment analysis. Nodes are keyed by their source span, so environments align by
  position, not by tree shape (the typechecker may rewrite the tree per environment). Policies
  that fail validation, template-linked policies and unlinked templates are skipped; so are
  condition-less policies (the typechecker folds them to `true`).
- Walking the spans in evaluation order: a span is reported when, in every environment that
  evaluates it, its outcome set lacks `True` (never true) or lacks `False` (never false), and no
  span inside it was already reported (the innermost cause is the warning; parents are implied).
  An environment that does not evaluate a span is ignored only when the span lies in the right
  operand of a conjunction whose left operand can never be true there (the policy is already
  false, what follows does not run); any other non-evaluation blocks the warning, which must
  speak for every environment. Literals the user wrote are not reported; a whole condition the
  typechecker folds to a literal is reported as the policy folded (the policy's own condition
  gives its `&&` chain the policy's source location, so a literal root with a location is
  treated as the whole policy).
- **Kept guards are not reported.** A `has`/`hasTag` guard of an optional attribute (or any
  tag) that a later access relies on is never false under an assumption saying the attribute is
  always present — and the warning would invite deleting a guard strict validation needs. The
  simplifier reads the evaluator's own judgement (`NodeOutcome::kept`, branch 5) instead of
  recomputing it: a span kept in any environment that evaluates it is not reported as never
  false (it stays reportable as never true or always an error), and it does not block its
  parent. `||` needs no rule: the validator gives the right operand of an `||` no capability
  from its left, so a `has` there guards nothing.
- `Finding { offset, length, kind, message }` — byte offsets into the policy text (so the
  input must be Cedar syntax), kinds `never-true`, `never-false`, `always-error`,
  `policy-always-false`, `policy-always-true`; `Serialize` with kebab-case kinds for editors.

## Files

- `cedar-policy-symcc/src/simplify.rs` (new), `src/lib.rs`, `Cargo.toml` (`serde`,
  `serde_json`), `tests/simplify.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test simplify` (cvc5): the `(a && b) || (a && !a)` example,
the innermost-cause rule, environments the policy cannot apply to, a conjunct that is false in
some environments wherever it sits, a disjunct not evaluated everywhere, a whole policy folded,
an atom that is true unless it errors, literals, a clean policy, invalid policies and templates;
the `@semantics` cases (the Kubernetes shape, unguarded, contradictory, all three placements);
the kept-guard cases (`principal has nick && principal.nick == "al"` under an assumption that
`nick` is present reports nothing; `principal has nick && principal.a` reports the guard; a
required attribute's `has` is reported; a guard whose scope folds away is judged by its operand).

## History

Merged from the extension's design note "Policy Simplifier and Semantic Linter" and its
implementation notes, and from the private plans "simplify.rs over TypecheckedPolicySet" and
"the simplifier does not report a has/hasTag guard a later access needs". The first
implementation shipped as a separate `cedar-symcc-simplify` binary, which branch 8 replaces
with `cedar lint`; the binary does not appear on this stack.
