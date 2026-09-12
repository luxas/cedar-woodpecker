# Plan 3 — `TypecheckedPolicySet`: typecheck once, evaluate many times

## Goal

Every consumer of a validated policy re-runs the typechecker on the policy's condition before
doing its work and throws the typed expression away: the symbolic evaluator (branch 5) typechecks
its target on every call, SymCC's `well_typed_policy` computes an `Expr<Option<Type>>` and
erases it, the DNF passes typecheck-and-discard, and the policy simplifier typechecks each policy
once per request environment per call. Work that evaluates the same policies over and over —
the simplifier, the DNF pipeline, policy synthesis — repeats that on cloned ASTs.

This branch adds, in `cedar-policy-core` and as a public newtype in `cedar-policy`, a policy set
typechecked **once** against a schema: for every executable policy (static or linked) and every
request environment of the schema it stores the typed condition with the link's slots
substituted, the possible boolean outcomes of that condition, and — with the `tpe` feature — the
TPE residual, which `constant_fold` interprets. A trait, `Typecheckable`, makes "this needs a
typed expression" generic: a bare `Expr` answers with the typecheck every consumer runs today; a
`TypecheckedPolicy` answers from its cache when the schema is the one it was built with.

## Design

### Core (`cedar-policy-core/src/typechecked.rs`, `pub mod typechecked`, not feature-gated)

- `typecheck_expr(expr, env, schema)` / `typecheck_policy(policy, env, schema)`: the
  expression typecheck consumers ran by hand until now — wrap the expression in a permit policy
  whose `when` it is (core exposes no expression-level typechecker), run
  `validate_entity_types_and_literals`, then `typecheck_by_single_request_env` in strict mode;
  `Success` and an error-free `Irrelevant` are accepted. For a policy the environment is linked to
  the policy's slot bindings. Errors: `TypecheckError::{NotWellTyped { errs }, RequestEnvNotFound,
  MissingType, UnboundSlot}`.
- `trait Typecheckable { fn untyped(&self) -> &Expr; fn typechecked(&self, env, schema) ->
  Result<Arc<Expr<Option<Type>>>, TypecheckError> }`, implemented for `Expr` (the default =
  `typecheck_expr`), `&T` and `Arc<T>`. The typed expression is semantically equivalent to the
  untyped one but may differ structurally: the strict typechecker rewrites `false && x` to
  `false`.
- `EvaluationOutcome` moves here from `tpe::residual` (re-exported at the old path) so the
  outcomes field exists without `tpe`; `all_outcomes()` is `{True, False, Error}`.
- `TypecheckedPolicy { policy: Arc<Policy>, condition: Arc<Expr>, envs: HashMap<RequestType,
  EnvData> }` with `EnvData { typed: Result<Arc<Expr<Option<Type>>>, StaticError>, residual:
  Arc<Residual> (tpe), outcomes: NESet<EvaluationOutcome> }`. `typed` is a `Result` on purpose:
  constant folding can reduce a whole condition to a static error, which no `Expr` expresses
  — `StaticError { ty }` records exactly that instead of keeping a stale expression. `residual`
  is not an `Option`: it is always built, uninterpreted, from the typed condition at
  construction, and the outcomes come from it (`possible_bool_outcomes`, branch 2) — or default
  to the always-sound `{True, False, Error}` without `tpe`.
- `TypecheckedPolicySet::new(policies: PolicySet, schema: Arc<ValidatorSchema>)`: validates the
  set (strict; templates included), then for every executable policy and every
  `unlinked_request_envs(Strict)` typechecks and substitutes slots (`substitute_slots`, the
  typed analogue of the residual's slot handling). The set owns its `PolicySet`, so the cache
  cannot go stale. Accessors: `policy_set`, `into_policy_set`, `schema`, `policies`,
  `policies_mut`, `get`, `get_mut`; on a policy `policy`, `id`, `condition`, `schema`,
  `request_types`, `typed_condition`, `outcomes`, `residual`, `same_schema`, `fold_env` (the
  setter the folds use, public because the symbolic fold lives in another crate).
- `impl Typecheckable for TypecheckedPolicy`: the cached `Arc` when the environment is a key,
  its `typed` is `Ok`, and the schema is the one used up front — pointer equality, then the
  structural `ValidatorSchema == ValidatorSchema` — otherwise the default typecheck (correct on
  any schema, just not free).
- `constant_fold(self) -> Self` (`tpe`): for each policy and environment, the residual is
  interpreted by TPE against the maximally unknown request of that environment (known
  principal/resource types and action, unknown ids and context, no entities); the outcomes are
  the interpreted residual's, and `typed` becomes `Residual::to_typed_expr`'s answer: a root
  `Error` ⇒ `Err(StaticError)`; an `Error` below the root ⇒ the previous typed condition is kept
  (the residual's synthetic `error()` call is no Cedar function); otherwise the folded typed
  expression. Concrete set/record values are typed from `Type::Set`/`Type::Record`; extension
  values from their constructor call over literal arguments.

### Public wrapper (`cedar-policy/src/api/typechecked.rs`)

`PolicySet::typecheck(&self, schema: &Schema) -> Result<TypecheckedPolicySet, TypecheckError>`,
newtypes `TypecheckedPolicySet` / `TypecheckedPolicy` (`RefCast`, `AsRef`) mirroring the core
API with the public `RequestEnv` as the environment key, `constant_fold` under `tpe`,
`impl Typecheckable for TypecheckedPolicy`. The typed expression and `Type` are core types in the
public signature, as SymCC's `WellTypedPolicy::policy()` already exposes core types.

### Deliberately not done

TPE's `policy_residual_map` and SymCC's `well_typed_policy` / `CompiledPolicy*` keep their own
typechecks; the symbolic evaluator adopts the trait in branch 5 and the simplifier in branch 7.

## Files

- `cedar-policy-core/src/typechecked.rs` (new), `src/lib.rs` (`pub mod typechecked`),
  `src/tpe/residual.rs` (`to_typed_expr`, the `EvaluationOutcome` re-export).
- `cedar-policy/src/api/typechecked.rs` (new), `src/api.rs`, `Cargo.toml` (`nonempty-collections`).

## Verification

Unit tests in both modules: construction and per-environment queries, invalid sets rejected,
`constant_fold` narrowing a condition to `{False}` in an environment where the scope cannot
match, the static-error case, cache hits versus the fallback on another schema. `cargo test`
(the `tpe` default of branch 1 compiles the folds).

## History

Merged from the private plan "TypecheckedPolicySet — typecheck once, evaluate many times" and
its two revisions; the symbolic-evaluator half of that plan (typed results, `symbolic_fold`) is
part of branch 5, the simplifier half of branch 7. Two design corrections came from review
during development: the residual is never an `Option`, and `typed` is a `Result` so that a
condition folded to a static error is visible rather than silently stale.
