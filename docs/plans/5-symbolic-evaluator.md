# Plan 5 — The symbolic evaluator

## Goal

Cedar has two evaluators: the concrete one, which needs every input, and the type-aware partial
evaluator (TPE), which tolerates missing parts of the request and the entity store but reasons
*structurally* — it cannot see that `principal.age > 18 && principal.age < 10` is never true.
The symbolic evaluator is the third: it takes an arbitrary boolean **assumption** about the
input (an expression, a policy set's decision, complete or partial entity data and requests)
and asks an SMT solver, through SymCC's encoding, what a target expression can still evaluate
to. Its result is the expression with every visited node annotated with its *exact* set of
possible outcomes (`True`, `False`, `Error`) and with determined nodes folded to literals.

It is the engine of everything that follows: the policy simplifier reports the nodes whose
outcome is determined in every request environment; the DNF pipeline prunes cubes with it;
policy synthesis folds cube conditions under a transition. This branch is the evaluator itself,
its assumptions API, its self-check, hand-derived tests, and the differential tests against the
concrete evaluator and TPE (cedar-spec branch 5 also carries the Lean model and proofs).

## The contract

Fix a schema, a request environment ρ with symbolic environment σ, a boolean-typed expression
`e` and assumptions `A`. Let `Envs(A)` be the concrete environments that conform to σ, satisfy
`A`, and are well formed for `e` and the assumptions. `evaluate` returns `r : Expr<EvaluationMetadata>`
such that, for every `env ∈ Envs(A)`:

- **(S1) outcome soundness** — the concrete outcome of `e` is in the root's outcome set, and
  recursively for every visited boolean-structure node relative to the trail under which it was
  visited;
- **(S2) simplification soundness** — `r` evaluates like `e`, the same value or both an error
  (the error kind is not preserved);
- **(C) exactness** — with an exact solver, the sets are exactly the outcomes reachable in
  `Envs(A)` (stronger than TPE's over-approximation; tested, not proved).

`Result::Err` is reserved for infrastructure: solver errors, ill-typed input, an unknown
environment, contradictory assumptions, the recursion limit — never for `e` erroring.

## Design

### Evaluation

- **Typed input.** The evaluator works on the *typechecked* expression, as TPE does: the strict
  typechecker rewrites structure (`false && r` becomes `false`; `if c then a else b` with a
  `Bool(True)` test becomes `ite(c, a, a)`), so the returned expression is the typechecked shape.
  Any `Typecheckable` (branch 3) is accepted: a bare `Expr` gets the wrapper-policy typecheck;
  a `TypecheckedPolicy` answers from its cache. Every node of the result carries its type in the
  metadata (`EvaluationMetadata::ty`, excluded from equality), so a fold needs no re-typecheck
  (`typed_of` recovers the typed expression).
- **The tree.** The typed expression is compiled once, bottom-up, into a tree whose nodes are
  `And`/`Or`/`If`/`Not` (boolean structure) or atoms, each with its own SMT term (SymCC's
  optimizing per-node compilers, plus `compile_not`) and footprint. Building children the way
  `compile` does keeps the evaluator from rejecting what SymCC accepts.
- **Queries.** An assumption is stored uncompiled and compiled lazily per environment. A query
  is `env asserts ++ extra ++ enforcement(footprints) ++ trail ++ [c]`, the hierarchy
  enforcement (acyclicity, pairwise transitivity — `enforce_footprint`) computed once per call
  over the union of all footprints. The first query checks the assumptions are satisfiable
  (`UnsatisfiableAssumptions` otherwise).
- **Atoms.** Three satisfiability questions under the trail — can the term be `some true`,
  `some false`, `none` — are mutually exclusive and exhaustive, so their answers *are* the exact
  set; a literal term needs none, and the error question is skipped when the compiler shows the
  atom cannot error. `{True}`/`{False}` fold to the literal.
- **Structure and the trail.** `l && r` evaluates `l`, then — only if `True` is in `l`'s set
  (trail gating: the extension must be satisfiable) — evaluates `r` under the trail extended with
  `term(l) = some true`, using the *original* child's term; the outcome sets combine by Cedar's
  three-valued table (`l = False ⇒ False`, `l = Error ⇒ Error`, `l = True ⇒ r`'s). `||`, `if`
  and `!` are the duals; an unvisited `if` branch is kept with the default `{True, False, Error}`
  metadata. A necessarily erroring operand stops the descent.
- **Entity existence.** SymCC's attribute maps are total, so `<entity>.attr` compiles to a
  never-erring term although concretely a missing entity errors (and a required `has` is
  `false`). The evaluator keeps an uninterpreted predicate `exists[E]` per entity type in
  asserts of its own: it holds for every entity of an assumed store, and for the receiver of
  every strict `getAttr`/`getTag` (and every `has`/`hasTag` that is true) of an expression the
  assumptions or the trail say evaluated without error. After the three outcome questions, each
  existence-sensitive access asks a fourth — can the receiver be an entity that does not exist —
  and, if so, adds `Error` (or `False` for a required `has`). Two constructs make the term take a
  *phantom* value on a missing entity (`iferror`'s first argument; a required `has` nested in a
  larger term): there the access adds what the atom can then be (the fallback's literal for a
  `getAttr`/`getTag` under `iferror(e, <literal>)` at the root, all three outcomes otherwise), and
  no fact is derived from a receiver containing an `iferror` or a `has`.
- **Kept guards.** A `has` of an optional attribute (or any `hasTag`) whose access occurs in the
  scope it guards — the right operand of an `&&` whose left operand contains the guard, the
  `then` branch of an `if` whose test does — is never folded to `true`: it is returned as itself
  with outcomes `{True}`, so the result still passes strict validation (an optional access needs
  its guard). Decided on the input before evaluation (`kept_guards`, `mark_kept`); the folded
  result exposes each node's `kept` flag to consumers (`evaluate_observed`, `NodeOutcome`).
- **Self-check.** `check_equivalent(e, r)` compiles `r` *without* typechecking it (a folded
  `has` leaves a bare optional access strict typing would reject) and asks whether
  `term(e) ≠ term(r)` is unsatisfiable under the assumptions — (S2) as one query; every test and
  DRT target runs it.
- **Trace.** Every solver query (its full assert list and answer), the base and the
  `check_equivalent` verdict can be recorded (`EvaluationTrace`) for the Lean replay.

### Assumptions (`assume.rs`)

`assume_expr` / `assume_expr_outcomes(e, S)` (an expression restricted to an outcome set;
the assumption must typecheck in every environment it is compiled for), `assume_policyset_decision(ps, Allow|Deny)`
(the compiled policy set's decision term), `assume_entities` / `assume_partial_entities`
(open world: only the entities given are constrained, attribute by attribute, tags and
ancestors included) and `assume_request` / `assume_partial_request`. An assumed `allow` of a
single-permit set proves that permit's receivers exist; a multi-permit `allow` proves nothing.

### `SymbolicFold`

`TypecheckedPolicySet::symbolic_fold(&mut evaluator)` (an extension trait in
`cedar-policy-symcc/src/typechecked.rs`) evaluates every policy's condition in every
environment under the evaluator's assumptions and stores the typed fold and its outcomes back
into the set (branch 3's `fold_env`); a static-error environment is left alone; the set's schema
must be the evaluator's.

### Exposed by SymCC for the evaluator

`compile_not` and `enforce_footprint` (n-ary siblings of what SymCC has per policy),
`pub(crate)` visibility of the `compiler`/`enforcer` modules, `typecheck_condition` delegating
to core's `typecheck_policy`, the structure-preserving `expr_util` helpers (`children`,
`with_children`, `map_data`, `erase`), and two accessors on TPE responses (`request_env`,
`schema`) plus public `Value::get_as_bool` / `stack_size_check` in core.

## Files

- `cedar-policy-symcc/src/evaluator/{mod,compile,assume,err}.rs`, `src/typechecked.rs`,
  `src/expr_util.rs`, `src/{err,lib,symcc}.rs`, `src/symccopt/{compiler,enforcer}.rs`,
  `tests/{evaluator,evaluator_assume,typechecked}.rs`.
- `cedar-policy-core/src/evaluator.rs`, `src/tpe/response.rs`; `cedar-policy/src/api/tpe.rs`.

## Verification

`tests/evaluator.rs` and `tests/evaluator_assume.rs` hold three-valued truth tables and
expected residuals derived by hand from Cedar's semantics — they are not snapshots — and every
expected residual is additionally checked against the input with `check_equivalent`, so a wrong
table is caught twice. `tests/typechecked.rs` covers `symbolic_fold`. In cedar-spec (branch 5):
`symcc-evaluator-concrete-drt` (a fully known, strongly well-formed store: the result must fold
to exactly the concrete outcome), `symcc-evaluator-open-drt` (stores that may be open: the
concrete outcome is in the set and the fold evaluates like the input up to the error kind),
`symcc-evaluator-tpe-drt` (the outcome set is a subset of the residual's, and residual and
result are solver-equivalent to the condition), and the Lean model with the replay DRT.

## History

Merged from the private plans "the symbolic evaluator" (Phases 1–2), "entity existence in the
symbolic evaluator's outcomes" (three revisions: a UID list, then the `exists` predicate, then
phantom values from review), the evaluator half of "TypecheckedPolicySet", and the kept-guard
rule of "the review of the woodpecker context plan". The original scheme of up to six
implication checks per atom was replaced by the three exclusive questions before the first
commit; the query-trace API was added for the Lean replay.
