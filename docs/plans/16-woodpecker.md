# Plan 16 — `cedar-woodpecker`: synthesizing the policies a privilege escalation implies

## Goal

A principal often gets more than the policies explicitly grant. In Kubernetes, someone who may
*create* pods can mount a namespace's secrets into a pod and read them, although no policy
grants `get secrets`. The rule for such an implicit grant is system-specific and is expressed as
a **transition function**: source request environments `(action_i, resource_i)`, a target
environment `(action', resource')`, and a Cedar condition `T` relating the intermediate
requests to the target one ("the secret is in the pod's namespace"). Given a policy set and a
transition, `cedar-woodpecker` synthesizes the *implied* policy over the target environment —
what the principal can effectively do — by combining the source permissions, applying `T`, and
eliminating every mention of the intermediate requests. The name: Levaillant's woodpecker is
found in cedar forests and punches holes in a dense, well-defended tree.

## Design

### The pipeline (`src/cubes.rs`, `src/escalate.rs`)

1. **Source cubes.** The policy set becomes allow-only cubes (`dnf::allow_cubes`, branch 15),
   typechecked once (`PolicySet::typecheck`), TPE-folded and symbolically folded per request
   environment under the schema's `@semantics` assumptions and optional entity data; every
   (cube, environment) whose outcomes contain `True` is a source cube. A scope-mismatched cube
   folds to `false` at no solver cost.
2. **The extended schema** (`src/schema.rs`). For a transition and a principal type, a copy of
   the schema gives the target action's context the required attributes `action{i}`,
   `resource{i}` and `context{i}` — the `i`-th intermediate request. Each is typed through a
   fresh common type (`__woodpecker_action{i}` = the source action's `Action` type,
   `__woodpecker_resource{i}` = the resource type, `__woodpecker_context{i}` = the source
   action's context verbatim) added to the namespace that *defines* what it names and referenced
   qualified from the target action's context, so every nested name resolves where it was
   written. The copy is made on the JSON schema form (`SchemaFragment::to_json_value` → edit →
   parse); the target action's own context, which gains the attributes, is the one thing that
   must be inlined, with its unqualified names qualified by the namespace it came from.
3. **Combination.** Each source cube's condition is renamed into the extended environment
   (`action ↦ context.action{i}`, `resource ↦ context.resource{i}`, `context ↦
   context.context{i}`), disjoined per source, conjoined over the sources, pinned with
   `context.action{i} == at_i` (the folded cube no longer states its action) and conjoined with
   `T` — which is typechecked on its own in the target environment first (an unguarded optional
   access in `T` is the user's error, reported as such). The policy `permit(principal is pt,
   action == at', resource is rt') when { … }` is split into cubes on the extended schema and
   folded again with an evaluator on the extended schema (the same solver: the base evaluator's
   compiler is taken out and re-wrapped; assumptions and entities re-registered). Every cube
   that can be `True` is an **escalation path**.
4. **Elimination** (`escalate::eliminate`). A path's conjuncts are rewritten to a condition
   over the original schema, sound by construction (every rewrite is implied by the path) and
   checked with the solver per path:
   - `dnf::rewrite_like` first, so exact patterns feed the classes;
   - **equivalence classes** (`src/equiv.rs`): `<expr> == <constant>` substitutes the constant
     everywhere else (a record literal simplifying under `.a`/`has a`, literal equalities
     folding); the error guards `<expr> == <expr>` and `true` are dropped; an expression free of
     the intermediate request is canonical for one mentioning it; among two that mention it the
     first is — each phase to a fixpoint, in that order. Substitution is structural and
     top-down (a replaced node is not descended into), applied only to the *other* conjuncts of
     the defining equality; a pair whose canonical contains the replaced term is skipped, which
     is what makes the loop terminate;
   - **sets and `in`** (`src/sets.rs`): the `in` conjuncts of one intermediate term are closed
     under transitivity (`a in X ∧ X in c ⇒ a in c`; `X in a ∧ ¬(X in b) ⇒ ¬(a in b)` only when
     `a` is syntactically an entity, since an attribute may be a set; `a in X ∧ ¬(b in X) ⇒
     ¬(b in a)`), then two conjuncts using the same intermediate set (or element) in set
     operations become one operation over the remaining operands by the 81-cell matrix of
     branch 13 — every cell is usable, since its result is *implied* by the pair with no domain
     hypothesis, the inexpressible cells by their proved over-approximations — best rewrite
     first (informative exact, exact under a hypothesis, over-approximation, `true`), the
     best-ranked pair globally at each step;
   - the classes once more, then every conjunct still mentioning the intermediate request
     becomes `true`; positive `has`/`hasTag` conjuncts are ordered first so a guard `T`
     supplied precedes the access it guards.
   The result is validated on the original schema (a failure is reported per path, not fatal)
   and checked sound: under the path assumed, the synthesized condition must not be `False`.
5. **Output.** Synthesized policies as Cedar text with a provenance comment (`@woodpecker("<name>:
   <source cube ids>")`, ids `woodpecker.{name}.{k}`), or `--json` (`Escalation`: transition,
   principal, source cube ids, path, policy text, `sound`, `problem`).

### CLI (`src/main.rs`)

```
cedar-woodpecker cubes    --schema <file> [--policies <file>|stdin] [--entities <file>] [--json]
cedar-woodpecker escalate --schema <file> [--policies <file>] --transitions <file> [--entities <file>] [--json]
```

The transition file is JSON: `{"transitions": [{"name", "principal"?: [types], "sources":
[{"action", "resource"}], "target": {"action", "resource"}, "when": "<Cedar>"}]}`; `when` may
use `principal`, `action`, `resource`, `context` (the target request) and
`context.action{i}` / `context.resource{i}` / `context.context{i}`.

### Deliberately not done

No completeness check; no parallel pruning; transitions that change the principal
(impersonation) are outside the transition signature; no Lean model of the rewrites (the
solver check per path guarantees soundness at runtime); no long bounds.

## Files

- `cedar-woodpecker/`: `Cargo.toml`, `README.md`, `src/{lib,main,cubes,escalate,equiv,sets,expr,schema,transition}.rs`,
  `tests/golden.rs`, `testdata/{e2e,equiv,expr,schema,sets,transition}/*`; the workspace member.

## Verification

`cargo test -p cedar-woodpecker` (cvc5): golden files per unit — expression helpers,
equivalence-class cases, every matrix cell and the `in` closure, schema extensions, transition
parsing — and end-to-end cases (`input.cedar`, `schema.cedarschema`, `transitions.json` →
`output.cedar`/`output.json`): the Kubernetes chain, two sources, contradicting sources, a
record literal, sets and labels, `in` across namespaces, context bridged/dropped/namespaced,
the target's own context, an ill-typed transition, an unguarded optional. `UPDATE_GOLDEN=1`
regenerates; every diff is reviewed. The set goldens were cross-checked against the matrix.

## History

Merged from the private plans "woodpecker — scaffold, DNF + symbolic pruning end-to-end,
revision-0 elimination", "equivalence-class rewrites in the elimination step", "set and in
rewrites in the elimination", "the intermediate requests' contexts", and the review of the last
(the schema extension by aliases in the defining namespaces, `T` typechecked on its own, guards
first and a per-path report). Revision 0 over-approximated every intermediate conjunct to
`true`; each later plan refined only the elimination step.
