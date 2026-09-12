/*
 * Copyright Cedar Contributors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! A symbolic Cedar evaluator: simplifies a boolean expression under logical
//! assumptions, using the SMT solver to decide which sub-expressions are
//! determined.
//!
//! See `README.md` in this directory for the idea and the roadmap.
//!
//! # Contract
//!
//! Fix a schema, a request environment ρ, a boolean-typed expression `e` that
//! is well typed in ρ, and the registered assumptions `A`. Let `Envs(A)` be
//! the set of concrete `(request, entities)` inputs of ρ that
//!
//! * have a *hierarchical* store (acyclic and transitively closed) that
//!   validates against the schema, and
//! * satisfy `A`.
//!
//! Entities referenced by the expressions, the request or entity data need
//! *not* exist in the store — see *Entity existence* below. (The Lean
//! theorems are stated for strongly well-formed inputs, where they do; the
//! open-store case is covered by the `symcc-evaluator-open-drt` target.)
//!
//! [`Evaluator::evaluate`] returns an [`Expr<EvaluationMetadata>`] `r` such
//! that for every input in `Envs(A)`:
//!
//! * **(S1)** the concrete outcome of `e` — `True`, `False` or `Error` — is a
//!   member of `r.data().outcomes()`; the same holds for every `&&`/`||`/`if`/
//!   `!` node the evaluator visited, relative to the trail of assumptions under
//!   which it was reached (`x` in `x && y` was true, etc.);
//! * **(S2)** evaluating `r` gives the same result as evaluating `e`, up to the
//!   *kind* of error (only "some error" is preserved);
//! * **(C)** given an exact solver, `r.data().outcomes()` is exactly the set of
//!   outcomes `e` takes over `Envs(A)` — unlike TPE's
//!   `Residual::possible_bool_outcomes`, which is an over-approximation;
//! * **(G)** a `has` of an optional attribute, or a `hasTag`, whose access
//!   follows in the scope it guards is never folded to the literal `true`
//!   (it is returned with outcomes `{True}` instead), so every access in `r`
//!   is guarded as strict validation requires (plan 5; see the README's
//!   *Kept guards*).
//!
//! `r` is built from the *typechecked* form of `e`, which may differ
//! structurally from `e` (`false && x` becomes `false`), as for TPE.
//!
//! `Err` is reserved for infrastructure failures (solver errors, ill-typed
//! input, an unknown request environment, contradictory assumptions, the
//! recursion limit). An expression that necessarily errors is a normal result
//! whose outcomes are `{Error}`.
//!
//! # Entity existence
//!
//! The symbolic store cannot express that an entity is absent: every entity
//! type's attribute map is a total function, so `<entity>.attr` compiles to
//! a term that never errors and `<entity> has attr` of a required attribute
//! to the literal `true` — while concretely a `getAttr`/`getTag` on a missing
//! entity errors and such a `has` is `false`. The evaluator tracks existence
//! itself with an uninterpreted predicate `exists[E]` per entity type, used
//! only in asserts it adds: it holds for every entity of an assumed store,
//! and for the receiver of every strict `getAttr`/`getTag` (one evaluated
//! whenever the enclosing expression is) of an expression that an assumption
//! or the trail says evaluated without error — and of every `has`/`hasTag`
//! known to be true (only a present entity has attributes or tags). After
//! the three outcome questions, each existence-sensitive access of an atom
//! asks whether its receiver can be an entity that does not exist; if it
//! can, the outcomes the atom may then take join the set: `Error` for a
//! plain `getAttr`/`getTag`, `False` for a required `has` that is the atom,
//! and — where the symbolic term takes a *phantom* value instead (under
//! `iferror`'s first argument, whose error is coalesced symbolically only;
//! a `has` nested in a larger term) — the fallback's literal (either boolean
//! for a `has` under it) or all three.
//! An access on a receiver containing an `iferror` or a `has` proves
//! nothing: symbolically it may denote a different entity.
//!
//! # Soundness argument (sketch)
//!
//! Every term is produced by the symbolic compiler, whose bisimulation theorem
//! (`Cedar/Thm/SymCC/Compiler.lean`) says it interprets to exactly the
//! concrete result on every strongly well-formed input. An atom's outcome set
//! is decided by three satisfiability checks (`t = some true`,
//! `t = some false`, `t = none`), which are mutually exclusive and exhaustive
//! for an `Option<Bool>` term, so it is exact when the solver is; an `unknown`
//! answer is an `Err`, never a wrong set. The sets of `&&`/`||`/`if`/`!` nodes
//! are combined by the short-circuit rules, with the right operand evaluated
//! under the extra assumption that the left one took the value that reaches
//! it — exactly the condition under which Cedar evaluates that operand — and
//! only when that assumption is satisfiable. Hierarchy enforcement constraints
//! (acyclicity, transitivity) are added for the union of all footprints in a
//! query; every such constraint holds on every strongly well-formed input
//! (`Cedar/Thm/SymCC/Enforcer/Enforce.lean`), so they only remove spurious
//! models.
//!
//! NOTE: This API is experimental and may change.

mod assume;
mod compile;

pub mod err;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use async_recursion::async_recursion;
use cedar_policy::{Decision, Entities, Request, RequestEnv, Schema};
use cedar_policy_core::ast::{Context, EntityUID, Expr, ExprBuilder, ExprKind, Literal, PolicySet};
use cedar_policy_core::evaluator::stack_size_check;
use cedar_policy_core::expr_builder::ExprBuilder as _;
use cedar_policy_core::tpe::entities::PartialEntities;
use cedar_policy_core::tpe::request::{PartialEntityUID, PartialRequest};
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_core::typechecked::Typecheckable;
use cedar_policy_core::validator::types::Type;
use nonempty_collections::{nes, NESet, NonEmptyIterator};
use smol_str::format_smolstr;

use crate::solver::Solver;
use crate::symcc::function::UnaryFunction;
use crate::symcc::op::Uuf;
use crate::symcc::term::TermPrim;
use crate::symcc::term_type::TermType;
use crate::symcc::type_abbrevs::{
    core_entity_type_into_entity_type, core_uid_into_uid, EntityType as SymEntityType,
};
use crate::symccopt::{compiler as symcompiler, enforcer::enforce_footprint, CompiledPolicySet};
use crate::term::Term;
use crate::term_factory as factory;
use crate::{Asserts, CedarSymCompiler, CompiledSchema, SymEnv};

use crate::expr_util::{children, map_data, with_children};
use compile::{
    build_tree, erase, existence_facts, kept_guards, mark_kept, rebuild_err, require_boolean,
    term_literal, typecheck, typecheck_assumption, with_data, Existence, Node, NodeKind,
};
pub use err::EvaluationError;
use err::Result;

/// The set of outcomes — `True`, `False`, `Error` — that a boolean
/// (sub-)expression can still take, attached to every node of the expression
/// [`Evaluator::evaluate`] returns.
///
/// Only nodes the evaluator visited carry precise information: the root, and
/// the `&&`/`||`/`if`/`!` chains reachable from it. Every other node (the
/// insides of an atom such as `principal.age > 18`, or a dead `if` branch)
/// carries [`EvaluationMetadata::default`], which is all three outcomes.
#[derive(Clone)]
pub struct EvaluationMetadata {
    outcomes: NESet<EvaluationOutcome>,
    /// The type the typechecker gave the node, when the node comes from an
    /// evaluation (see [`typed_of`]); not part of equality.
    ty: Option<Type>,
}

/// Equality is on the outcomes: the type is a fact about the node, not a
/// result of the evaluation.
impl PartialEq for EvaluationMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.outcomes == other.outcomes
    }
}

impl Eq for EvaluationMetadata {}

impl EvaluationMetadata {
    /// Metadata with the given outcomes (and no type).
    pub fn new(outcomes: NESet<EvaluationOutcome>) -> Self {
        Self { outcomes, ty: None }
    }

    /// The possible outcomes.
    pub fn outcomes(&self) -> &NESet<EvaluationOutcome> {
        &self.outcomes
    }

    /// The type the typechecker gave the node, when known.
    pub fn ty(&self) -> Option<&Type> {
        self.ty.as_ref()
    }

    /// The same outcomes, with `ty` as the node's type.
    fn with_ty(self, ty: Option<Type>) -> Self {
        Self { ty, ..self }
    }

    /// Whether `o` is a possible outcome.
    pub fn can(&self, o: &EvaluationOutcome) -> bool {
        self.outcomes.contains(o)
    }

    /// Whether the only possible outcome is an error.
    pub fn is_error(&self) -> bool {
        self.outcomes.len().get() == 1 && self.can(&EvaluationOutcome::Error)
    }

    /// Whether an error is impossible.
    pub fn is_error_free(&self) -> bool {
        !self.can(&EvaluationOutcome::Error)
    }

    fn literal(b: bool) -> Self {
        Self::new(if b {
            nes![EvaluationOutcome::True]
        } else {
            nes![EvaluationOutcome::False]
        })
    }

    fn from_flags(can_true: bool, can_false: bool, can_error: bool) -> Option<Self> {
        let mut set: Option<NESet<EvaluationOutcome>> = None;
        for (flag, o) in [
            (can_true, EvaluationOutcome::True),
            (can_false, EvaluationOutcome::False),
            (can_error, EvaluationOutcome::Error),
        ] {
            if flag {
                match &mut set {
                    Some(s) => {
                        s.insert(o);
                    }
                    None => set = Some(nes![o]),
                }
            }
        }
        set.map(Self::new)
    }

    /// Outcomes of `!x` given those of `x`.
    fn negated(&self) -> Self {
        Self::new(
            self.outcomes
                .nonempty_iter()
                .map(|o| match o {
                    EvaluationOutcome::True => EvaluationOutcome::False,
                    EvaluationOutcome::False => EvaluationOutcome::True,
                    EvaluationOutcome::Error => EvaluationOutcome::Error,
                })
                .collect(),
        )
    }

    /// Outcomes of `l && r`, where `r`'s outcomes were computed under the
    /// assumption that `l` is true.
    fn and(&self, right: &Self) -> Self {
        Self::new(
            self.outcomes
                .nonempty_iter()
                .flat_map(|o| match o {
                    EvaluationOutcome::True => right.outcomes.clone(),
                    EvaluationOutcome::False => nes![EvaluationOutcome::False],
                    EvaluationOutcome::Error => nes![EvaluationOutcome::Error],
                })
                .collect(),
        )
    }

    /// Outcomes of `l || r`, where `r`'s outcomes were computed under the
    /// assumption that `l` is false.
    fn or(&self, right: &Self) -> Self {
        Self::new(
            self.outcomes
                .nonempty_iter()
                .flat_map(|o| match o {
                    EvaluationOutcome::False => right.outcomes.clone(),
                    EvaluationOutcome::True => nes![EvaluationOutcome::True],
                    EvaluationOutcome::Error => nes![EvaluationOutcome::Error],
                })
                .collect(),
        )
    }

    /// Outcomes of `if c then a else b`, where `a`/`b` were computed under the
    /// assumption that `c` is true/false. A branch is `None` when `c` cannot
    /// take the value that reaches it.
    fn ite(&self, then: Option<&Self>, els: Option<&Self>) -> Self {
        let set: Vec<EvaluationOutcome> = self
            .outcomes
            .iter()
            .flat_map(|o| match o {
                EvaluationOutcome::True => then.map(|m| m.outcomes.iter().cloned().collect()),
                EvaluationOutcome::False => els.map(|m| m.outcomes.iter().cloned().collect()),
                EvaluationOutcome::Error => Some(vec![EvaluationOutcome::Error]),
            })
            .flatten()
            .collect();
        Self::from_flags(
            set.contains(&EvaluationOutcome::True),
            set.contains(&EvaluationOutcome::False),
            set.contains(&EvaluationOutcome::Error),
        )
        // `self` is non-empty and every branch that is consulted is non-empty,
        // so the union is non-empty.
        .unwrap_or_default()
    }
}

impl Default for EvaluationMetadata {
    /// All three outcomes: the least informative value, and the only one that
    /// is sound for a node the evaluator did not look at.
    fn default() -> Self {
        Self::new(nes![
            EvaluationOutcome::True,
            EvaluationOutcome::False,
            EvaluationOutcome::Error
        ])
    }
}

impl fmt::Debug for EvaluationMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for EvaluationMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (EvaluationOutcome::True, "True"),
            (EvaluationOutcome::False, "False"),
            (EvaluationOutcome::Error, "Error"),
        ]
        .into_iter()
        .filter(|(o, _)| self.can(o))
        .map(|(_, n)| n)
        .collect::<Vec<_>>();
        write!(f, "{{{}}}", names.join(", "))
    }
}

/// One query the evaluator issued: the full assert list passed to
/// `check_unsat_raw` (base, trail and constraint concatenated) and its answer.
#[derive(Clone, Debug)]
pub struct QueryRecord {
    /// The asserts, in query order.
    pub asserts: Vec<Term>,
    /// Whether they were unsatisfiable.
    pub unsat: bool,
}

/// A recording of one [`Evaluator::evaluate`] call (and, when it follows, the
/// [`Evaluator::check_equivalent`] call for the same expression), for
/// replaying through the Lean model of the evaluator.
#[derive(Clone, Debug)]
pub struct EvaluationTrace {
    /// The typechecked-then-erased target expression — the tree the evaluator
    /// actually walks, which may differ structurally from the input.
    pub erased: Expr,
    /// The base asserts of the `evaluate` call: assumptions, extras and
    /// hierarchy enforcement, in query order.
    pub base: Vec<Term>,
    /// The base asserts of the `check_equivalent` call, if one was recorded.
    pub ce_base: Option<Vec<Term>>,
    /// Every query, in order: `evaluate`'s (starting with the up-front base
    /// check) followed by `check_equivalent`'s.
    pub queries: Vec<QueryRecord>,
}

/// The literal boolean value of an evaluated expression, if it is one.
pub fn literal_of(expr: &Expr<EvaluationMetadata>) -> Option<bool> {
    match expr.expr_kind() {
        ExprKind::Lit(Literal::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Strips the metadata from an evaluated expression, e.g. to use it as a
/// policy condition or to compare it with a parsed expression.
pub fn erase_metadata(expr: &Expr<EvaluationMetadata>) -> Result<Expr> {
    erase(expr)
}

/// The typed form of an evaluated expression: every node with the type the
/// typechecker gave it (see [`EvaluationMetadata::ty`]), preserving structure
/// exactly. Fails on an expression not produced by an evaluation (a node
/// without a type, as [`with_default_metadata`] makes them).
pub fn typed_of(expr: &Expr<EvaluationMetadata>) -> Result<Expr<Option<Type>>> {
    let typed = map_data(expr, &|md: &EvaluationMetadata| md.ty.clone()).map_err(rebuild_err)?;
    if all_nodes(&typed).any(|n| n.data().is_none()) {
        return Err(EvaluationError::Internal(
            "an evaluated expression has a node without a type",
        ));
    }
    Ok(typed)
}

/// Every node of `expr`, root first.
fn all_nodes<T>(expr: &Expr<T>) -> impl Iterator<Item = &Expr<T>> {
    let mut stack = vec![expr];
    std::iter::from_fn(move || {
        let e = stack.pop()?;
        stack.extend(children(e));
        Some(e)
    })
}

/// Rebuilds a typed expression with `root` at the root and the default
/// metadata at every other node, each node carrying its own type.
fn zip_types(
    expr: &Expr<Option<Type>>,
    root: EvaluationMetadata,
) -> Result<Expr<EvaluationMetadata>> {
    stack_size_check().map_err(|_| EvaluationError::RecursionLimit)?;
    let new = children(expr)
        .into_iter()
        .map(|c| zip_types(c, EvaluationMetadata::default()))
        .collect::<Result<Vec<_>>>()?;
    with_children(expr, new, root.with_ty(expr.data().clone())).map_err(rebuild_err)
}

/// Annotates every node of `expr` with [`EvaluationMetadata::default`] (all
/// three outcomes), preserving its structure exactly. This is the shape
/// [`Evaluator::check_equivalent`] accepts for the expression it compiles
/// without typechecking, so an expression produced elsewhere (a TPE residual,
/// say) can be checked against a target expression under the assumptions.
pub fn with_default_metadata(expr: &Expr) -> Result<Expr<EvaluationMetadata>> {
    with_data(expr, EvaluationMetadata::default())
}

/// A term of type `Bool` that holds iff the `Option<Bool>` term `t` takes one
/// of `outcomes`.
fn outcome_constraint(t: &Term, outcomes: &NESet<EvaluationOutcome>) -> Term {
    let mut constraint: Term = false.into();
    if outcomes.contains(&EvaluationOutcome::True) {
        constraint = factory::or(
            constraint,
            factory::eq(t.clone(), factory::some_of(true.into())),
        );
    }
    if outcomes.contains(&EvaluationOutcome::False) {
        constraint = factory::or(
            constraint,
            factory::eq(t.clone(), factory::some_of(false.into())),
        );
    }
    if outcomes.contains(&EvaluationOutcome::Error) {
        constraint = factory::or(constraint, factory::is_none(t.clone()));
    }
    constraint
}

/// An assumption, kept uncompiled because it has to be compiled once per
/// request environment.
enum Assumption {
    /// The expression takes one of the outcomes.
    ExprOutcomes(Arc<Expr>, NESet<EvaluationOutcome>),
    /// The policy set produces the decision.
    PolicySetDecision(Arc<PolicySet>, Decision),
    /// The entity store agrees with the (partial) entities.
    Entities(Arc<PartialEntities>),
    /// The request agrees with the (partial) request.
    Request(Arc<PartialRequest>),
}

/// Everything compiled for one request environment.
struct EnvState {
    sym_env: SymEnv,
    /// One `Bool` term per compiled assumption.
    asserts: Vec<Term>,
    /// Union of the footprints of the compiled assumptions.
    footprint: BTreeSet<Term>,
    /// How many of `Evaluator::assumptions` have been compiled into this env.
    compiled_upto: usize,
}

impl EnvState {
    fn compile_assumption(
        &mut self,
        assumption: &Assumption,
        req_env: &RequestEnv,
        schema: &Schema,
    ) -> Result<()> {
        match assumption {
            Assumption::ExprOutcomes(expr, outcomes) => {
                let typed = typecheck_assumption(expr, req_env, schema)?;
                let res = symcompiler::compile(&erase(&typed)?, &self.sym_env)
                    .map_err(crate::Error::from)?;
                self.asserts.push(outcome_constraint(&res.term, outcomes));
                self.footprint.extend(res.footprint);
                // an expression that does not error has evaluated its strict
                // `getAttr`/`getTag` receivers: they exist (and, when its
                // value is known, its `has`/`hasTag` receivers too)
                if !outcomes.contains(&EvaluationOutcome::Error) {
                    let truth = if outcomes.len().get() == 1 {
                        Some(outcomes.contains(&EvaluationOutcome::True))
                    } else {
                        None
                    };
                    self.asserts
                        .extend(existence_fact_terms(&typed, &self.sym_env, truth));
                }
            }
            Assumption::PolicySetDecision(pset, decision) => {
                let compiled = CompiledPolicySet::compile_with_custom_symenv(
                    pset,
                    req_env,
                    schema,
                    self.sym_env.clone(),
                )
                .map_err(|e| match e {
                    crate::Error::PolicyNotWellTyped { errs } => {
                        EvaluationError::AssumptionNotWellTyped {
                            env: req_env.clone(),
                            errs,
                        }
                    }
                    other => EvaluationError::SymCC(other),
                })?;
                self.asserts.push(match decision {
                    Decision::Allow => compiled.term,
                    Decision::Deny => factory::not(compiled.term),
                });
                self.footprint.extend(compiled.footprint);
                // `allow` with a single permit means that permit evaluated
                // to `true`: its strict receivers exist. (With several
                // permits no single one is known to have evaluated; `deny`
                // says nothing either way.)
                let permits: Vec<_> = pset
                    .policies()
                    .filter(|p| p.effect() == cedar_policy_core::ast::Effect::Permit)
                    .collect();
                if let (Decision::Allow, [permit]) = (decision, permits.as_slice()) {
                    let typed = typecheck_assumption(&permit.condition(), req_env, schema)?;
                    self.asserts
                        .extend(existence_fact_terms(&typed, &self.sym_env, Some(true)));
                }
            }
            Assumption::Entities(entities) => {
                let asserted = assume::entities_asserts(entities, &self.sym_env, schema)?;
                self.asserts.extend(asserted.asserts);
                self.footprint.extend(asserted.footprint);
                // the entities exist
                for entity in entities.entities() {
                    let uid = entity.uid();
                    self.asserts.push(exists(
                        core_entity_type_into_entity_type(uid.entity_type()),
                        Term::Prim(TermPrim::Entity(core_uid_into_uid(uid).clone())),
                    ));
                }
            }
            Assumption::Request(request) => {
                let asserted = assume::request_asserts(request, req_env, &self.sym_env, schema)?;
                self.asserts.extend(asserted.asserts);
                self.footprint.extend(asserted.footprint);
            }
        }
        Ok(())
    }
}

/// The symbolic evaluator. Bound to a schema; assumptions are registered with
/// the `assume_*` methods and apply to every subsequent evaluation, in every
/// request environment.
#[derive(Debug)]
pub struct Evaluator<S: Solver> {
    compiler: CedarSymCompiler<S>,
    schema: Schema,
    compiled_schema: CompiledSchema,
    assumptions: Vec<Assumption>,
    envs: HashMap<RequestEnv, EnvState>,
    solver_queries: usize,
    recording: bool,
    trace: Option<EvaluationTrace>,
}

impl fmt::Debug for Assumption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExprOutcomes(e, o) => write!(
                f,
                "ExprOutcomes({e}, {})",
                EvaluationMetadata::new(o.clone())
            ),
            Self::PolicySetDecision(p, d) => write!(f, "PolicySetDecision({p}, {d:?})"),
            Self::Entities(e) => write!(f, "Entities({e:?})"),
            Self::Request(r) => write!(f, "Request({r:?})"),
        }
    }
}

impl fmt::Debug for EnvState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvState")
            .field("asserts", &self.asserts)
            .field("footprint", &self.footprint)
            .field("compiled_upto", &self.compiled_upto)
            .finish_non_exhaustive()
    }
}

impl<S: Solver + Send> Evaluator<S> {
    /// Constructs an evaluator for `schema`.
    pub fn new(compiler: CedarSymCompiler<S>, schema: &Schema) -> crate::err::Result<Self> {
        Ok(Self {
            compiler,
            schema: schema.clone(),
            compiled_schema: CompiledSchema::new(schema)?,
            assumptions: Vec::new(),
            envs: HashMap::new(),
            solver_queries: 0,
            recording: false,
            trace: None,
        })
    }

    /// The underlying compiler (and through it, the solver).
    pub fn compiler_mut(&mut self) -> &mut CedarSymCompiler<S> {
        &mut self.compiler
    }

    /// Consumes the evaluator and returns the compiler it was built with, so
    /// a solver process can outlive one evaluator (e.g. across fuzz inputs).
    pub fn into_compiler(self) -> CedarSymCompiler<S> {
        self.compiler
    }

    /// The schema the evaluator is bound to.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Number of solver queries issued so far (constant-folded queries that
    /// never reached the solver are not counted).
    pub fn solver_queries(&self) -> usize {
        self.solver_queries
    }

    /// Starts or stops recording an [`EvaluationTrace`]. While recording,
    /// each `evaluate` call begins a fresh trace (also recording the
    /// `check_equivalent` call that follows it, if any); retrieve it with
    /// [`Self::take_trace`].
    pub fn record_trace(&mut self, on: bool) {
        self.recording = on;
        if !on {
            self.trace = None;
        }
    }

    /// Takes the trace recorded since the last `evaluate` call.
    pub fn take_trace(&mut self) -> Option<EvaluationTrace> {
        self.trace.take()
    }

    /// Assumes that `expr` evaluates to `true`.
    pub fn assume_expr(&mut self, expr: Expr) {
        self.assume_expr_outcomes(expr, nes![EvaluationOutcome::True]);
    }

    /// Assumes that `expr` evaluates to one of `outcomes`.
    ///
    /// The expression must be well typed (strict mode) in every request
    /// environment that is later evaluated; otherwise evaluation there fails
    /// with [`EvaluationError::AssumptionNotWellTyped`]. Use the
    /// `extra_assumptions` argument of [`Self::evaluate`] for assumptions that
    /// only make sense in some environments.
    pub fn assume_expr_outcomes(&mut self, expr: Expr, outcomes: NESet<EvaluationOutcome>) {
        self.assumptions
            .push(Assumption::ExprOutcomes(Arc::new(expr), outcomes));
        // Compilation into already-prepared environments is lazy; see `prepare_env`.
    }

    /// Assumes that authorizing with `policies` yields `decision`.
    pub fn assume_policyset_decision(&mut self, policies: PolicySet, decision: Decision) {
        self.assumptions
            .push(Assumption::PolicySetDecision(Arc::new(policies), decision));
    }

    /// Assumes that the entity store agrees with `entities` on every entity
    /// in it (attributes, ancestors and tags, each where the partial entity
    /// provides them). Entities not in `entities` stay unconstrained.
    ///
    /// `entities` must be validated against the schema, which
    /// `PartialEntities::from_concrete` / `from_entities` guarantee.
    pub fn assume_partial_entities(&mut self, entities: PartialEntities) {
        self.assumptions
            .push(Assumption::Entities(Arc::new(entities)));
    }

    /// Checks that `expr` can be an assumption in `req_env`: well typed
    /// (strict mode) and boolean. What [`Self::evaluate`]'s extra assumptions
    /// require of every environment they are used in.
    pub fn typecheck_assumption(&self, expr: &Expr, req_env: &RequestEnv) -> Result<()> {
        let typed = typecheck_assumption(expr, req_env, &self.schema)?;
        require_boolean(&typed)
    }

    /// Assumes that the entity store contains exactly the data in `entities`
    /// for the entities in it (see [`Self::assume_partial_entities`]).
    pub fn assume_entities(&mut self, entities: &Entities) -> Result<()> {
        let partial =
            PartialEntities::from_concrete(entities.as_ref().clone(), self.schema.as_ref())
                .map_err(|e| EvaluationError::InvalidEntities(e.to_string()))?;
        self.assume_partial_entities(partial);
        Ok(())
    }

    /// Assumes that the request agrees with `request` on the parts it
    /// provides (principal and resource ids, context). In a request
    /// environment whose principal type, action or resource type differ from
    /// the request's, the assumption is `false`.
    pub fn assume_partial_request(&mut self, request: PartialRequest) {
        self.assumptions
            .push(Assumption::Request(Arc::new(request)));
    }

    /// Assumes that the request is exactly `request` (see
    /// [`Self::assume_partial_request`]). The request must be concrete.
    pub fn assume_request(&mut self, request: &Request) -> Result<()> {
        let invalid = |what: &str| EvaluationError::InvalidRequest(format!("{what} is unknown"));
        let principal: &EntityUID = request
            .principal()
            .ok_or_else(|| invalid("principal"))?
            .as_ref();
        let action: &EntityUID = request.action().ok_or_else(|| invalid("action"))?.as_ref();
        let resource: &EntityUID = request
            .resource()
            .ok_or_else(|| invalid("resource"))?
            .as_ref();
        let context = match request
            .context()
            .ok_or_else(|| invalid("context"))?
            .as_ref()
        {
            Context::Value(attrs) => attrs.clone(),
            _ => return Err(invalid("context")),
        };
        let partial = PartialRequest::new(
            PartialEntityUID {
                ty: principal.entity_type().clone(),
                eid: Some(principal.eid().clone()),
            },
            action.clone(),
            PartialEntityUID {
                ty: resource.entity_type().clone(),
                eid: Some(resource.eid().clone()),
            },
            Some(context),
            self.schema.as_ref(),
        )
        .map_err(|e| EvaluationError::InvalidRequest(e.to_string()))?;
        self.assume_partial_request(partial);
        Ok(())
    }

    /// Symbolically evaluates `expr` in `req_env` under the registered
    /// assumptions and `extra_assumptions` (each assumed to be `true`, for this
    /// call only). See the module documentation for the contract.
    pub async fn evaluate<T: Typecheckable + ?Sized>(
        &mut self,
        expr: &T,
        req_env: &RequestEnv,
        extra_assumptions: impl IntoIterator<Item = Expr>,
    ) -> Result<Expr<EvaluationMetadata>> {
        self.evaluate_inner(expr, req_env, extra_assumptions, None)
            .await
    }

    /// [`Self::evaluate`], also returning the outcome set of *every* node the
    /// evaluator visited, in evaluation order (a node after its children),
    /// each with its source location and whether it is a kept guard. The
    /// returned expression folds nodes whose sets are singletons (kept
    /// guards excepted); this keeps what was known about them.
    pub async fn evaluate_observed<T: Typecheckable + ?Sized>(
        &mut self,
        expr: &T,
        req_env: &RequestEnv,
        extra_assumptions: impl IntoIterator<Item = Expr>,
    ) -> Result<(Expr<EvaluationMetadata>, Vec<NodeOutcome>)> {
        let mut observed = Vec::new();
        let result = self
            .evaluate_inner(expr, req_env, extra_assumptions, Some(&mut observed))
            .await?;
        Ok((result, observed))
    }

    async fn evaluate_inner<T: Typecheckable + ?Sized>(
        &mut self,
        expr: &T,
        req_env: &RequestEnv,
        extra_assumptions: impl IntoIterator<Item = Expr>,
        observed: Option<&mut Vec<NodeOutcome>>,
    ) -> Result<Expr<EvaluationMetadata>> {
        self.prepare_env(req_env)?;
        let state = self
            .envs
            .get(req_env)
            .ok_or(EvaluationError::Internal("request env not prepared"))?;
        let typed = typecheck(expr, req_env, &self.schema)?;
        require_boolean(&typed)?;
        let mut footprint = state.footprint.clone();
        let mut root = build_tree(&typed, &state.sym_env, &mut footprint)?;
        mark_kept(&mut root, &kept_guards(&typed, &state.sym_env));
        let mut base = state.asserts.clone();
        Self::add_extra_assumptions(
            &self.schema,
            state,
            req_env,
            extra_assumptions,
            &mut base,
            &mut footprint,
        )?;
        base.extend(enforce_footprint(&footprint, &state.sym_env.entities));
        if self.recording {
            self.trace = Some(EvaluationTrace {
                erased: erase(&typed)?,
                base: base.clone(),
                ce_base: None,
                queries: Vec::new(),
            });
        }

        let mut ctx = Ctx {
            compiler: &mut self.compiler,
            sym_env: &state.sym_env,
            base,
            queries: 0,
            log: self.trace.as_mut().map(|t| &mut t.queries),
            observed,
        };
        let result = async {
            if ctx.unsat(&[], None).await? {
                return Err(EvaluationError::UnsatisfiableAssumptions);
            }
            let mut trail = Vec::new();
            eval_node(&mut ctx, &root, &mut trail).await
        }
        .await;
        self.solver_queries += ctx.queries;
        result
    }

    /// Checks that `result` (as returned by [`Self::evaluate`] for `expr`) is
    /// equivalent to `expr` on every input in `Envs(A)` — contract (S2) as a
    /// single solver query. Intended for tests and differential testing.
    ///
    /// `result` is compiled as is, without typechecking it again: a folded
    /// `principal has foo` may have left a `principal.foo` behind that only
    /// typechecks with the capability the `has` provided.
    pub async fn check_equivalent<T: Typecheckable + ?Sized>(
        &mut self,
        expr: &T,
        result: &Expr<EvaluationMetadata>,
        req_env: &RequestEnv,
        extra_assumptions: impl IntoIterator<Item = Expr>,
    ) -> Result<bool> {
        self.prepare_env(req_env)?;
        let state = self
            .envs
            .get(req_env)
            .ok_or(EvaluationError::Internal("request env not prepared"))?;
        let typed = typecheck(expr, req_env, &self.schema)?;
        require_boolean(&typed)?;
        let mut footprint = state.footprint.clone();
        let root = build_tree(&typed, &state.sym_env, &mut footprint)?;
        let res =
            symcompiler::compile(&erase(result)?, &state.sym_env).map_err(crate::Error::from)?;
        footprint.extend(res.footprint);
        let mut base = state.asserts.clone();
        Self::add_extra_assumptions(
            &self.schema,
            state,
            req_env,
            extra_assumptions,
            &mut base,
            &mut footprint,
        )?;
        base.extend(enforce_footprint(&footprint, &state.sym_env.entities));
        if let Some(trace) = self.trace.as_mut().filter(|_| self.recording) {
            trace.ce_base = Some(base.clone());
        }

        let mut ctx = Ctx {
            compiler: &mut self.compiler,
            sym_env: &state.sym_env,
            base,
            queries: 0,
            log: self.trace.as_mut().map(|t| &mut t.queries),
            observed: None,
        };
        let differ = factory::not(factory::eq(root.term.clone(), res.term));
        let result = ctx.unsat(&[], Some(differ)).await;
        self.solver_queries += ctx.queries;
        result
    }

    /// Makes sure `req_env` has a `SymEnv` and that every registered
    /// assumption has been compiled for it.
    fn prepare_env(&mut self, req_env: &RequestEnv) -> Result<()> {
        if !self.envs.contains_key(req_env) {
            let sym_env = self
                .compiled_schema
                .sym_env(req_env)
                .map_err(|_| EvaluationError::RequestEnvNotFound(req_env.clone()))?;
            self.envs.insert(
                req_env.clone(),
                EnvState {
                    sym_env,
                    asserts: Vec::new(),
                    footprint: BTreeSet::new(),
                    compiled_upto: 0,
                },
            );
        }
        let state = self
            .envs
            .get_mut(req_env)
            .ok_or(EvaluationError::Internal("request env not prepared"))?;
        while let Some(assumption) = self.assumptions.get(state.compiled_upto) {
            state.compile_assumption(assumption, req_env, &self.schema)?;
            state.compiled_upto += 1;
        }
        Ok(())
    }

    fn add_extra_assumptions(
        schema: &Schema,
        state: &EnvState,
        req_env: &RequestEnv,
        extra_assumptions: impl IntoIterator<Item = Expr>,
        base: &mut Vec<Term>,
        footprint: &mut BTreeSet<Term>,
    ) -> Result<()> {
        for extra in extra_assumptions {
            let typed = typecheck_assumption(&extra, req_env, schema)?;
            let res = symcompiler::compile(&erase(&typed)?, &state.sym_env)
                .map_err(crate::Error::from)?;
            base.push(factory::eq(res.term, factory::some_of(true.into())));
            footprint.extend(res.footprint);
            // it evaluated to `true`: its receivers exist
            base.extend(existence_fact_terms(&typed, &state.sym_env, Some(true)));
        }
        Ok(())
    }
}

/// The request environment a concrete request belongs to, if its action is in
/// the schema.
pub fn request_env_of(request: &Request, schema: &Schema) -> Option<RequestEnv> {
    let principal = request.principal()?;
    let action = request.action()?;
    let resource = request.resource()?;
    schema.as_ref().get_action_id(action.as_ref())?;
    Some(RequestEnv::new(
        principal.type_name().clone(),
        action.clone(),
        resource.type_name().clone(),
    ))
}

/// The outcome set of one node the evaluator visited, with the node's
/// source location (see [`Evaluator::evaluate_observed`]).
#[derive(Debug, Clone)]
pub struct NodeOutcome {
    /// Where the node is in the source, if known.
    pub loc: Option<cedar_policy_core::parser::Loc>,
    /// Whether the node is a literal (as written, or as the typechecker
    /// folded it).
    pub literal: bool,
    /// The node's outcomes under the trail it was reached with.
    pub outcomes: NESet<EvaluationOutcome>,
    /// Whether the node is a `has`/`hasTag` guard the evaluator keeps (plan
    /// 025): of an optional attribute or a tag, with its access in the scope
    /// it guards — one strict validation needs, however true it is.
    pub kept: bool,
}

/// Everything one evaluation needs to talk to the solver.
struct Ctx<'a, S: Solver> {
    compiler: &'a mut CedarSymCompiler<S>,
    sym_env: &'a SymEnv,
    /// Assumptions, extra assumptions and enforcement constraints.
    base: Vec<Term>,
    queries: usize,
    /// Where to record every query, when tracing.
    log: Option<&'a mut Vec<QueryRecord>>,
    /// Where to record every visited node's outcomes, when observing.
    observed: Option<&'a mut Vec<NodeOutcome>>,
}

/// `exists[E](t)`: the evaluator's own uninterpreted existence predicate for
/// the entity type `ety`, applied to the entity term `t`. The symbolic store
/// has no notion of an entity being absent (its attribute maps are total),
/// so the evaluator tracks existence itself: the predicate appears only in
/// asserts it adds — facts for the entities it is told exist and for the
/// receivers of accesses known to have evaluated — and in its existence
/// questions.
fn exists(ety: &SymEntityType, t: Term) -> Term {
    let uuf = Uuf {
        id: format_smolstr!("exists[{ety}]"),
        arg: TermType::Entity { ety: ety.clone() },
        out: TermType::Bool,
    };
    factory::app(UnaryFunction::Uuf(Arc::new(uuf)), t)
}

/// The fact that `check`'s receiver exists, `exists[E](option_get(r))`.
fn exists_fact(check: &Existence) -> Term {
    exists(&check.ety, factory::option_get(check.receiver.clone()))
}

/// The existence facts of `x` as an assumption that it evaluated as
/// `truth` says (`Some(b)`: to `b`; `None`: without error).
fn existence_fact_terms(
    typed: &Expr<Option<Type>>,
    sym_env: &SymEnv,
    truth: Option<bool>,
) -> Vec<Term> {
    existence_facts(typed, sym_env, truth)
        .iter()
        .map(exists_fact)
        .collect()
}

impl<S: Solver + Send> Ctx<'_, S> {
    /// Whether `base ∧ trail ∧ constraint` is unsatisfiable.
    async fn unsat(&mut self, trail: &[Term], constraint: Option<Term>) -> Result<bool> {
        let asserts: Asserts = Arc::new(
            self.base
                .iter()
                .cloned()
                .chain(trail.iter().cloned())
                .chain(constraint)
                .collect(),
        );
        // `check_unsat_raw` short-circuits on constant asserts without a
        // solver round-trip; counting those as queries would be misleading,
        // so mirror its two checks here.
        if !asserts.iter().any(|a| *a == false.into()) && !asserts.iter().all(|a| *a == true.into())
        {
            self.queries += 1;
        }
        let unsat = self
            .compiler
            .check_unsat_raw(asserts.clone(), self.sym_env)
            .await?;
        if let Some(log) = self.log.as_deref_mut() {
            log.push(QueryRecord {
                asserts: asserts.as_ref().clone(),
                unsat,
            });
        }
        Ok(unsat)
    }

    /// The outcome set of an atom with term `term` under `trail`: the three
    /// outcome questions, then one existence question per access in
    /// `checks` whose outcome is not yet in the set.
    async fn atom_outcomes(
        &mut self,
        term: &Term,
        checks: &[Existence],
        trail: &[Term],
    ) -> Result<EvaluationMetadata> {
        let mut outcomes = self.term_outcomes(term, trail).await?.outcomes;
        for check in checks {
            if check.adds.iter().all(|o| outcomes.contains(o)) {
                continue;
            }
            if !self.unsat(trail, Some(self.missing(check))).await? {
                outcomes.extend(check.adds.iter().cloned());
            }
        }
        Ok(EvaluationMetadata::new(outcomes))
    }

    /// The constraint that `check`'s receiver is an entity that does not
    /// exist: `is_some(r) && !exists[E](option_get(r))`.
    fn missing(&self, check: &Existence) -> Term {
        factory::and(
            factory::is_some(check.receiver.clone()),
            factory::not(exists_fact(check)),
        )
    }

    /// The exact outcome set of the `Option<Bool>` term `term` under `trail`.
    async fn term_outcomes(&mut self, term: &Term, trail: &[Term]) -> Result<EvaluationMetadata> {
        if let Some(outcome) = term_literal(term) {
            return Ok(EvaluationMetadata::new(nes![outcome]));
        }
        let can_true = !self
            .unsat(
                trail,
                Some(factory::eq(term.clone(), factory::some_of(true.into()))),
            )
            .await?;
        let can_false = !self
            .unsat(
                trail,
                Some(factory::eq(term.clone(), factory::some_of(false.into()))),
            )
            .await?;
        let is_none = factory::is_none(term.clone());
        let can_error = if is_none == false.into() {
            false
        } else {
            !self.unsat(trail, Some(is_none)).await?
        };
        EvaluationMetadata::from_flags(can_true, can_false, can_error).ok_or(
            EvaluationError::Internal("no possible outcome for an atom under a satisfiable trail"),
        )
    }
}

fn md(expr: &Expr<EvaluationMetadata>) -> &EvaluationMetadata {
    expr.data()
}

fn literal_expr(node: &Node, b: bool) -> Expr<EvaluationMetadata> {
    ExprBuilder::with_data(EvaluationMetadata::literal(b).with_ty(node.expr.data().clone()))
        .with_same_source_loc(&*node.expr)
        .val(Literal::Bool(b))
}

fn structure_expr(
    node: &Node,
    metadata: EvaluationMetadata,
    kind: ExprKind<EvaluationMetadata>,
) -> Expr<EvaluationMetadata> {
    ExprBuilder::with_data(metadata.with_ty(node.expr.data().clone()))
        .with_same_source_loc(&*node.expr)
        .with_expr_kind(kind)
}

/// A kept guard (plan 5) whose scope folded to a literal guards nothing
/// any more: the literal `true` it stands for. Any other value is returned
/// as is (an *atom* with outcomes `{True}` that is not a literal is always a
/// kept guard — every other such atom was folded; a structure node with
/// those outcomes is left alone, as in the Lean model's `SEExpr.unguard`).
fn unguard(node: &Node, v: Expr<EvaluationMetadata>) -> Expr<EvaluationMetadata> {
    if matches!(node.kind, NodeKind::Atom { .. })
        && literal_of(&v).is_none()
        && *md(&v) == EvaluationMetadata::literal(true)
    {
        literal_expr(node, true)
    } else {
        v
    }
}

/// Pushes onto `trail` that `node` evaluated to `some b`, followed by the
/// existence facts that follow from it having evaluated without error;
/// returns how many terms were pushed.
fn push_trail(trail: &mut Vec<Term>, node: &Node, b: bool) -> usize {
    trail.push(factory::eq(node.term.clone(), factory::some_of(b.into())));
    let facts = node.facts.get(Some(b));
    trail.extend(facts.iter().map(exists_fact));
    1 + facts.len()
}

/// A child the evaluator did not visit, annotated with the default metadata.
fn unvisited(node: &Node) -> Result<Expr<EvaluationMetadata>> {
    zip_types(&node.expr, EvaluationMetadata::default())
}

/// The evaluation proper. `trail` holds the assumptions that follow from the
/// path taken to `node` (`term(x) = some true` for the right operand of
/// `x && …`, etc.) and is always satisfiable together with `ctx.base`.
#[expect(
    clippy::multiple_bound_locations,
    reason = "`async_recursion` adds a `Send` bound of its own"
)]
#[async_recursion]
async fn eval_node<S: Solver + Send>(
    ctx: &mut Ctx<'_, S>,
    node: &Node,
    trail: &mut Vec<Term>,
) -> Result<Expr<EvaluationMetadata>> {
    let result = eval_node_inner(ctx, node, trail).await?;
    if let Some(observed) = ctx.observed.as_deref_mut() {
        // the residual's root carries exactly this node's outcome set (a
        // folded literal's is the singleton)
        observed.push(NodeOutcome {
            loc: node.expr.source_loc().cloned(),
            literal: matches!(node.expr.expr_kind(), ExprKind::Lit(_)),
            outcomes: result.data().outcomes().clone(),
            kept: matches!(node.kind, NodeKind::Atom { keep: true, .. }),
        });
    }
    Ok(result)
}

#[expect(
    clippy::multiple_bound_locations,
    reason = "`async_recursion` adds a `Send` bound of its own"
)]
#[async_recursion]
async fn eval_node_inner<S: Solver + Send>(
    ctx: &mut Ctx<'_, S>,
    node: &Node,
    trail: &mut Vec<Term>,
) -> Result<Expr<EvaluationMetadata>> {
    stack_size_check().map_err(|_| EvaluationError::RecursionLimit)?;
    match &node.kind {
        NodeKind::Atom { checks, keep } => {
            let outcomes = ctx.atom_outcomes(&node.term, checks, trail).await?;
            // a kept guard stays itself, with its `{True}` outcomes (plan 5)
            Ok(if outcomes == EvaluationMetadata::literal(true) && !*keep {
                literal_expr(node, true)
            } else if outcomes == EvaluationMetadata::literal(false) {
                literal_expr(node, false)
            } else {
                zip_types(&node.expr, outcomes)?
            })
        }
        NodeKind::Not(x) => {
            let xv = eval_node(ctx, x, trail).await?;
            if let Some(b) = literal_of(&xv) {
                return Ok(literal_expr(node, !b));
            }
            if md(&xv).is_error() {
                return Ok(xv);
            }
            let metadata = md(&xv).negated();
            Ok(structure_expr(
                node,
                metadata,
                ExprKind::UnaryApp {
                    op: cedar_policy_core::ast::UnaryOp::Not,
                    arg: Arc::new(xv),
                },
            ))
        }
        NodeKind::And(l, r) => {
            let lv = eval_node(ctx, l, trail).await?;
            if md(&lv).is_error() {
                return Ok(lv);
            }
            match literal_of(&lv) {
                Some(false) => return Ok(literal_expr(node, false)),
                Some(true) => {
                    let r = r.as_ref().ok_or(EvaluationError::Internal(
                        "right operand of `&&` missing although the left one is true",
                    ))?;
                    return eval_node(ctx, r, trail).await;
                }
                None => {}
            }
            if !md(&lv).can(&EvaluationOutcome::True) {
                // `l && r` is `l` whenever `l` cannot be true.
                return Ok(lv);
            }
            let r = r.as_ref().ok_or(EvaluationError::Internal(
                "right operand of `&&` missing although the left one can be true",
            ))?;
            let pushed = push_trail(trail, l, true);
            let rv = eval_node(ctx, r, trail).await;
            trail.truncate(trail.len() - pushed);
            let rv = rv?;
            match literal_of(&rv) {
                Some(true) => Ok(unguard(l, lv)),
                Some(false) if md(&lv).is_error_free() => Ok(literal_expr(node, false)),
                _ => {
                    let metadata = md(&lv).and(md(&rv));
                    Ok(structure_expr(
                        node,
                        metadata,
                        ExprKind::And {
                            left: Arc::new(lv),
                            right: Arc::new(rv),
                        },
                    ))
                }
            }
        }
        NodeKind::Or(l, r) => {
            let lv = eval_node(ctx, l, trail).await?;
            if md(&lv).is_error() {
                return Ok(lv);
            }
            match literal_of(&lv) {
                Some(true) => return Ok(literal_expr(node, true)),
                Some(false) => {
                    let r = r.as_ref().ok_or(EvaluationError::Internal(
                        "right operand of `||` missing although the left one is false",
                    ))?;
                    return eval_node(ctx, r, trail).await;
                }
                None => {}
            }
            if !md(&lv).can(&EvaluationOutcome::False) {
                // `l || r` is `l` whenever `l` cannot be false.
                return Ok(lv);
            }
            let r = r.as_ref().ok_or(EvaluationError::Internal(
                "right operand of `||` missing although the left one can be false",
            ))?;
            let pushed = push_trail(trail, l, false);
            let rv = eval_node(ctx, r, trail).await;
            trail.truncate(trail.len() - pushed);
            let rv = rv?;
            match literal_of(&rv) {
                Some(false) => Ok(lv),
                Some(true) if md(&lv).is_error_free() => Ok(literal_expr(node, true)),
                _ => {
                    let metadata = md(&lv).or(md(&rv));
                    Ok(structure_expr(
                        node,
                        metadata,
                        ExprKind::Or {
                            left: Arc::new(lv),
                            right: Arc::new(rv),
                        },
                    ))
                }
            }
        }
        NodeKind::If(c, a, b) => {
            let cv = eval_node(ctx, c, trail).await?;
            if md(&cv).is_error() {
                return Ok(cv);
            }
            match literal_of(&cv) {
                Some(true) => {
                    let a = a.as_ref().ok_or(EvaluationError::Internal(
                        "then-branch missing although the test is true",
                    ))?;
                    return eval_node(ctx, a, trail).await;
                }
                Some(false) => {
                    let b = b.as_ref().ok_or(EvaluationError::Internal(
                        "else-branch missing although the test is false",
                    ))?;
                    return eval_node(ctx, b, trail).await;
                }
                None => {}
            }
            let (a, b) = match (a, b) {
                (Some(a), Some(b)) => (a, b),
                _ => {
                    return Err(EvaluationError::Internal(
                        "branch missing although the test is not a literal",
                    ))
                }
            };
            let av = if md(&cv).can(&EvaluationOutcome::True) {
                let pushed = push_trail(trail, c, true);
                let av = eval_node(ctx, a, trail).await;
                trail.truncate(trail.len() - pushed);
                Some(av?)
            } else {
                None
            };
            let bv = if md(&cv).can(&EvaluationOutcome::False) {
                let pushed = push_trail(trail, c, false);
                let bv = eval_node(ctx, b, trail).await;
                trail.truncate(trail.len() - pushed);
                Some(bv?)
            } else {
                None
            };
            // a kept guard as the test, with a `then` branch that folded to
            // a literal, guards nothing any more (plan 5)
            if let Some(av) = &av {
                if literal_of(av).is_some() && *md(&cv) == EvaluationMetadata::literal(true) {
                    return Ok(av.clone());
                }
            }
            let metadata = md(&cv).ite(av.as_ref().map(md), bv.as_ref().map(md));
            let av = match av {
                Some(av) => av,
                None => unvisited(a)?,
            };
            let bv = match bv {
                Some(bv) => bv,
                None => unvisited(b)?,
            };
            Ok(structure_expr(
                node,
                metadata,
                ExprKind::If {
                    test_expr: Arc::new(cv),
                    then_expr: Arc::new(av),
                    else_expr: Arc::new(bv),
                },
            ))
        }
    }
}
