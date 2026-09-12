# Plan 14 — Splitting policies: one policy per cube

## Goal

A policy `permit(…) when { a || b }` applies in two distinct ways: when `a` holds, and when
`a` fails and `b` holds. Policy synthesis and the analyses that follow want each way as its own
policy, so that a cube — one self-contained condition — can be renamed, combined and folded
independently. This branch lifts the DNF pipeline to whole policies: `split_policy` makes every
true-cube of the DNF of a policy's condition its own policy with the same effect, scope and
annotations; `split_policy_set` replaces every policy of a set by its split. The authorization
decision is preserved exactly; only the diagnostics may differ.

## Design

- **Why it is sound.** The decision of a policy set depends only on which policies evaluate to
  `true` — an erroring policy is ignored, like a false one. On any input the original condition
  is `true` iff exactly one true-cube is (branch 9's exclusivity), so replacing a policy by its
  split preserves every decision, for permits and forbids alike. Never-true cubes exist only to
  reproduce errors and can be dropped entirely (`Dnf::of(…, |_| false, …)`), so `can_error` is
  irrelevant here. Scope stays in place: `condition = scope && when`, so splitting only the
  `when` and copying the scope onto every split policy preserves each policy's matching set and
  keeps the output validatable and sliceable per request environment. What is *not* preserved:
  where the original policy errors, the split may contain fewer erroring policies (a dropped
  never-true cube) or more (cubes sharing the erroring prefix each error).
- **The pipeline.** `split_policy(policy, schema, max_split_nodes, max_cubes)`: validate the
  policy against the schema (strict — the elimination's rewrites assume typing; a linked policy
  is validated as its static instance, not its template), normalize the condition
  (`normalize_unchecked`: split → eliminate → split, branch 12), convert with `Dnf::of(…, |_|
  false, …)`, and make each true-cube `i` a `StaticPolicy` with id `{id}.cube{i}`, the original's
  location, annotations, effect and *filled* scope constraints (so template-linked policies split
  into equivalent static policies for free). Zero cubes (`when { false }`, a contradiction) make
  the policy vanish; an empty cube renders as `when { true }`; a policy without a `when` clause
  splits as `when { true }`. Generated ids cannot collide (`{id}.cube{i}` is injective and input
  ids are unique), so `PolicySet::add`'s occupied-id error is unreachable. Unlinked templates are
  not carried over.
- Budgets are charged afresh per policy.

## Files

- `cedar-policy-symcc/src/dnf/policy.rs` (new), `src/dnf/mod.rs` (the Step 3 doc, the
  re-exports), `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf`: hand tables (`a || b` → `[a, !a && b]` with ids,
effect, annotations and scope preserved; the worked example; `when { false }` → nothing;
scope-only → `when { true }`; a forbid splits as a forbid), decision truth tables (over every
rule-table entry and every small shape: the original is `T` iff exactly one split is, never two),
solver-checked authorization equivalence of original and split sets, the concrete `Authorizer`
on real entities (an erroring forbid ignored on both sides, a true forbid denying on both),
template links, budgets, the adversarial ids `{p, p.cube0}`, validation of the static instance of
a linked policy, and the error-kind cases of the split and elimination passes through the
concrete `Authorizer`. The Lean model, the decision theorem and the `split-policies-lean-drt`
target are cedar-spec branch 14.

## History

Merged from the private plan "split policies (Phase 3, Step 3)" and the later changes that
made `split_policy` validate against a schema and normalize the condition.
