# Plan 12 — Eliminating record and set literals from atoms

## Goal

After the DNF conversion and the atom split (branches 9, 10) every atom is an opaque term, but
an atom may still hide structure under a *literal*: a record literal under `.attr`, `has` or
`==`, a set literal under `contains`, `containsAll`, `containsAny`, `isEmpty` or `in`. The
equivalence-class and quantifier-elimination passes of policy synthesis want to see through
those — `{a: x}.a` is `x`, `[l₁, l₂].contains(x)` is `l₁ == x || l₂ == x`, `e in [g₁, g₂]` is
`e in g₁ || e in g₂`. This branch adds `eliminate_aggregates`, which rewrites them away,
bottom-up in every atom, keeping evaluation exact with the splitter's guards, and
`normalize_atoms`, the pipeline split → eliminate → split.

## Design

### The rules (children already rewritten)

```
{a: x, …}.a            ⇒ x                       (the first field named a)
{a: x, …} has a        ⇒ true / false
{a: x, …} == {a: y, …} ⇒ x == y && …              (same keys)
a == b, a set literal on either side
                        ⇒ a.containsAll(b) && b.containsAll(a)
[l₁, …, lₙ].contains(x) ⇒ l₁ == x || … || lₙ == x
s.containsAll([l₁, …])  ⇒ s.contains(l₁) && …
s.containsAny([l₁, …])  ⇒ s.contains(l₁) || …      (and symmetrically)
[l₁, …].isEmpty()       ⇒ false / true
e in [x₁, …, xₙ]        ⇒ e in x₁ || … || e in xₙ
```

An equality, `contains` or `containsAll` a rule produces is rewritten again when it meets
literals (`{a: {b: 1}} == {a: {b: 2}}` ends as `1 == 2`; `s == [l₁, l₂]` as
`(s.contains(l₁) && s.contains(l₂)) && [l₁, l₂].containsAll(s)`); the empty chains are `true`
and `false`. A literal survives only where no rule reaches it: `<set>.contains(<record>)`, a
record literal on one side of `==`, a set literal on the *left* of `containsAll`. Set equality
holds for *well-formed* sets — two set values are equal exactly when each contains the other —
which a schema-conformant store's values and every evaluated literal are.

### Exactness through guards

A rewrite reorders and drops evaluation (`{a: x, b: y}.b` never evaluates `x` once it is `y`),
so every rewritten atom becomes `if (g₁ == g₁ && …) then atom' else false` where the guards are,
in evaluation order, the subterms the original evaluated: every maximal unrewritten subterm (a
*leaf region*, decomposed like a split guard) and, for an unrewritten node above a rewritten
one, the node itself after rewriting (its own error must still precede its right siblings');
rewritten nodes never err once their children have evaluated. Guards the rewritten atom
evaluates itself, first and in the same order — the longest suffix of the guard list that is a
prefix of the atom's own evaluation order, after the dedupe against the context — are dropped
again, so the common rules leave no wrapper (`[1, 2].contains(p.age)` is plainly `1 == p.age ||
2 == p.age`). The pass walks the structure with the splitter's context (the first split's
wrapper guards, the enclosing conjuncts' evaluated closures), so it repeats no guard the
structure already established, and the second split learns the elimination wrappers' guards in
turn. An `iferror` call and any `&&`/`||`/`!`/`if` inside an atom are opaque leaf regions, so
the pass is exact on any input but *complete* only on split input — hence the pipeline.

### Typing is required, and enforced

`in` over a set type-checks *every* element before answering while the disjunction stops at the
first match; `containsAll`/`containsAny` on a non-set left operand would type-error where the
chain of `contains` calls does not; `==` with a set literal against a non-set is `false` where
the `containsAll` pair type-errors. So the rewrites assume a well-typed input and the entry
points enforce it: `eliminate_aggregates` and `normalize_atoms` typecheck the expression against
the schema in its request environment (the evaluator's `typecheck_expr`), and an ill-typed
input is rejected (`DnfError::NotWellTyped`, `RequestEnvNotFound`, `Typecheck`) rather than
rewritten. Typing is also why no guard other than `<expr> == <expr>` is ever needed.

### Budget

The `contains`-family rules duplicate an operand per element (shared, so the elimination is
linear; exponential as a tree under nesting). The pass has its own node budget: each rewritten
atom's wrapper is charged its tree size computed on the shared structure, so an output too large
for the next split is rejected in time linear in the shared size.

## Files

- `cedar-policy-symcc/src/dnf/elim.rs` (new), `src/dnf/mod.rs` (the module doc, the typing
  error variants, `or_chain`), `src/dnf/split.rs` (the shared guard helpers are `pub(super)`),
  `src/evaluator/mod.rs` (`typecheck_expr`), `tests/dnf.rs`.

## Verification

`cargo test -p cedar-policy-symcc --test dnf`: the elimination table (exact forms,
elimination-freedom via the `eliminated` predicate, solver equivalence), the budgets, an
ill-typed input rejected, `elim_rejects_ill_typed_input`. The Lean model and proofs and the
`elim-lean-drt` target are cedar-spec branch 12; the concrete-authorizer error-kind cases
(`elim_preserves_error_kind`) arrive with branch 14's fixtures.

## History

Merged from the private plan "eliminate record and set literals in atoms (Phase 4 Step 1)" and
its seven revisions: the guard-context improvements, the elimination's own budget, dropping the
guards a term evaluates first (from review), and set equality with a literal side, which was
held back until typing was enforced.
