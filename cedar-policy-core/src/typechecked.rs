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

//! Policies typechecked once, in every request environment of a schema, so
//! that the consumers of typed conditions (the symbolic evaluator, later TPE
//! and SymCC) do not re-run the typechecker on every use.
//!
//! [`TypecheckedPolicySet`] owns a [`PolicySet`] and the schema it was
//! validated against; for every executable policy (static or linked) and every
//! request environment it stores the typed condition
//! ([`Expr<Option<Type>>`]), with the link's slots substituted, the possible
//! boolean outcomes of that condition — always sound: without the `tpe`
//! feature the default `{True, False, Error}`, with it the outcomes of the
//! condition's [`Residual`] — and, with `tpe`, the residual itself, which
//! [`TypecheckedPolicySet::constant_fold`] interprets.
//!
//! [`Typecheckable`] is how a consumer asks for a typed expression: for a bare
//! [`Expr`] the default implementation is the typecheck every consumer runs
//! today ([`typecheck_expr`]); a [`TypecheckedPolicy`] answers from its cache
//! whenever the schema is the one it was built with.

use std::collections::HashMap;
use std::sync::Arc;

use linked_hash_map::LinkedHashMap;
use miette::Diagnostic;
use nonempty_collections::{nes, NESet};
use thiserror::Error;

use crate::ast::{
    Effect, Expr, ExprBuilder, ExprKind, Literal, Policy, PolicyID, PolicySet, RequestType,
    SlotEnv, SlotId, Template,
};
use crate::expr_builder::ExprBuilder as _;
use crate::validator::typecheck::{PolicyCheck, Typechecker};
use crate::validator::types::{RequestEnv, Type};
use crate::validator::{ValidationError, ValidationMode, Validator, ValidatorSchema};

#[cfg(feature = "tpe")]
use crate::tpe::residual::Residual;

/// The possible outcomes of evaluating a boolean expression.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum EvaluationOutcome {
    /// The expression evaluates to `true`
    True,
    /// The expression evaluates to `false`
    False,
    /// The expression evaluates to some error
    Error, // TODO: Split into RuntimeError, EntityNotExistError and TypeError?
}

/// Every outcome: what is known about a boolean expression before any
/// analysis. Always sound.
pub fn all_outcomes() -> NESet<EvaluationOutcome> {
    nes![
        EvaluationOutcome::True,
        EvaluationOutcome::False,
        EvaluationOutcome::Error
    ]
}

/// Why a policy or expression could not be typechecked.
#[derive(Debug, Clone, Error, Diagnostic)]
pub enum TypecheckError {
    /// The policy (or expression) is not well typed in some request environment.
    #[error("policy is not well typed")]
    NotWellTyped {
        /// The validation errors
        #[related]
        errs: Vec<ValidationError>,
    },
    /// The typed expression has a node without a type (a typechecker recovery
    /// the caller should not see).
    #[error("typed expression has a node without a type")]
    MissingType,
    /// A template slot the policy does not bind.
    #[error("slot {0} is not bound by the policy")]
    UnboundSlot(SlotId),
}

/// The typechecker's answer, as the consumers here read it: `Success` and an
/// error-free `Irrelevant` (the policy never applies in this environment, but
/// is well typed) are typed expressions; anything else is an error.
fn typecheck_template(
    template: &Template,
    env: &RequestEnv<'_>,
    schema: &ValidatorSchema,
) -> Result<Expr<Option<Type>>, TypecheckError> {
    // The checks the validator does besides typechecking; the existence of
    // `env` stands in for `validate_template_action_application`.
    let errs: Vec<_> = Validator::validate_entity_types_and_literals(schema, template).collect();
    if !errs.is_empty() {
        return Err(TypecheckError::NotWellTyped { errs });
    }
    let typechecker = Typechecker::new(schema, ValidationMode::Strict);
    match typechecker.typecheck_by_single_request_env(template, env) {
        PolicyCheck::Success(expr) => Ok(expr),
        PolicyCheck::Irrelevant(errs, expr) => {
            if errs.is_empty() {
                Ok(expr)
            } else {
                Err(TypecheckError::NotWellTyped { errs })
            }
        }
        PolicyCheck::Fail(errs) => Err(TypecheckError::NotWellTyped { errs }),
    }
}

/// Typechecks a bare expression in `env` (strict mode) and returns the typed
/// expression: the check every consumer runs today.
///
/// The typechecker works on policies, so `expr` is wrapped in a `permit`
/// policy whose condition is `true && (true && (true && expr))`; the returned
/// expression is that condition, typed. The typechecker may rewrite the
/// expression (`false && x` becomes `false`), so the result is semantically
/// equivalent to `expr` but not necessarily structurally identical.
pub fn typecheck_expr(
    expr: &Expr,
    env: &RequestEnv<'_>,
    schema: &ValidatorSchema,
) -> Result<Expr<Option<Type>>, TypecheckError> {
    let wrapper = Policy::from_when_clause(
        Effect::Permit,
        expr.clone(),
        PolicyID::from_string("__typecheck"),
        None,
    );
    typecheck_template(wrapper.template(), env, schema)
}

/// Typechecks the condition of `policy` in `env` (strict mode), with the
/// environment linked to the policy's slot bindings, and returns the typed
/// condition (`P && (A && (R && when))`, slots not substituted).
pub fn typecheck_policy(
    policy: &Policy,
    env: &RequestEnv<'_>,
    schema: &ValidatorSchema,
) -> Result<Expr<Option<Type>>, TypecheckError> {
    let env = env.clone().link_slot_env(policy.env());
    typecheck_template(policy.template(), &env, schema)
}

/// Something that can be typechecked in a request environment.
///
/// The default [`Typecheckable::typechecked`] is [`typecheck_expr`] on
/// [`Typecheckable::untyped`] — what every consumer does today. A
/// [`TypecheckedPolicy`] overrides it with its cached typed condition.
pub trait Typecheckable {
    /// The untyped expression.
    fn untyped(&self) -> &Expr;

    /// The typed expression in `env` under `schema`: semantically equivalent
    /// to [`Typecheckable::untyped`] (its structure may differ, as the
    /// typechecker rewrites `false && x` to `false`).
    fn typechecked(
        &self,
        env: &RequestEnv<'_>,
        schema: &ValidatorSchema,
    ) -> Result<Arc<Expr<Option<Type>>>, TypecheckError> {
        typecheck_expr(self.untyped(), env, schema).map(Arc::new)
    }
}

impl Typecheckable for Expr {
    fn untyped(&self) -> &Expr {
        self
    }
}

impl<T: Typecheckable + ?Sized> Typecheckable for &T {
    fn untyped(&self) -> &Expr {
        (**self).untyped()
    }

    fn typechecked(
        &self,
        env: &RequestEnv<'_>,
        schema: &ValidatorSchema,
    ) -> Result<Arc<Expr<Option<Type>>>, TypecheckError> {
        (**self).typechecked(env, schema)
    }
}

impl<T: Typecheckable + ?Sized> Typecheckable for Arc<T> {
    fn untyped(&self) -> &Expr {
        (**self).untyped()
    }

    fn typechecked(
        &self,
        env: &RequestEnv<'_>,
        schema: &ValidatorSchema,
    ) -> Result<Arc<Expr<Option<Type>>>, TypecheckError> {
        (**self).typechecked(env, schema)
    }
}

/// Folding established that a condition always errors in an environment. An
/// error is not expressible as an [`Expr`], so where the typed condition
/// would be, this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticError {
    /// The type the condition had.
    pub ty: Type,
}

/// What is known about one policy in one request environment.
#[derive(Debug, Clone)]
struct EnvData {
    /// The typed condition, slots substituted, every node typed; `Err` once a
    /// fold reduced the whole condition to an error.
    typed: Result<Arc<Expr<Option<Type>>>, StaticError>,
    /// The condition as a residual: at construction converted as is (no
    /// interpretation); after [`TypecheckedPolicySet::constant_fold`], the
    /// interpreted residual.
    #[cfg(feature = "tpe")]
    residual: Arc<Residual>,
    /// The possible outcomes of the condition; always sound (an
    /// over-approximation): the residual's, or every outcome without `tpe`.
    outcomes: NESet<EvaluationOutcome>,
}

impl EnvData {
    fn new(typed: Arc<Expr<Option<Type>>>) -> Result<Self, TypecheckError> {
        #[cfg(feature = "tpe")]
        {
            let residual = Residual::try_from_typed_expr(&typed, &SlotEnv::new())
                .map_err(|_| TypecheckError::MissingType)?;
            let outcomes = residual.possible_bool_outcomes();
            Ok(Self {
                typed: Ok(typed),
                residual: Arc::new(residual),
                outcomes,
            })
        }
        #[cfg(not(feature = "tpe"))]
        {
            Ok(Self {
                typed: Ok(typed),
                outcomes: all_outcomes(),
            })
        }
    }

    /// Records an interpreted residual: its outcomes, and its typed form where
    /// it has one (see [`Residual::to_typed_expr`]).
    #[cfg(feature = "tpe")]
    fn fold_residual(&mut self, residual: Residual) {
        self.outcomes = residual.possible_bool_outcomes();
        match residual.to_typed_expr() {
            Err(err) => self.typed = Err(err),
            Ok(Some(expr)) => self.typed = Ok(Arc::new(expr)),
            Ok(None) => {}
        }
        self.residual = Arc::new(residual);
    }
}

/// One policy, typechecked in every request environment of a schema.
#[derive(Debug, Clone)]
pub struct TypecheckedPolicy {
    policy: Arc<Policy>,
    /// `policy.condition()` with the link's slots substituted.
    condition: Arc<Expr>,
    /// The schema the conditions were typechecked against.
    schema: Arc<ValidatorSchema>,
    envs: HashMap<RequestType, EnvData>,
}

impl TypecheckedPolicy {
    /// Typechecks `policy` in every request environment of `schema`.
    ///
    /// `policy` must pass validation against `schema` (see
    /// [`TypecheckedPolicySet::new`]); this only typechecks.
    pub fn new(policy: Arc<Policy>, schema: Arc<ValidatorSchema>) -> Result<Self, TypecheckError> {
        let condition = Arc::new(substitute_slots(&policy.condition(), policy.env())?);
        let mut envs = HashMap::new();
        for env in schema.unlinked_request_envs(ValidationMode::Strict) {
            let Some(request_type) = env.to_request_type() else {
                continue;
            };
            let typed = typecheck_policy(&policy, &env, &schema)?;
            let typed = substitute_slots(&typed, policy.env())?;
            envs.insert(request_type, EnvData::new(Arc::new(typed))?);
        }
        Ok(Self {
            policy,
            condition,
            schema,
            envs,
        })
    }

    /// The policy.
    pub fn policy(&self) -> &Arc<Policy> {
        &self.policy
    }

    /// The policy's id.
    pub fn id(&self) -> &PolicyID {
        self.policy.id()
    }

    /// The policy's condition, with the link's slots substituted.
    pub fn condition(&self) -> &Arc<Expr> {
        &self.condition
    }

    /// The schema the conditions were typechecked against.
    pub fn schema(&self) -> &Arc<ValidatorSchema> {
        &self.schema
    }

    /// The request environments the policy is typechecked in (every
    /// environment of the schema).
    pub fn request_types(&self) -> impl Iterator<Item = &RequestType> {
        self.envs.keys()
    }

    /// The typed condition in `request_type`, or the static error a fold
    /// reduced it to; `None` for an environment not of the schema.
    pub fn typed_condition(
        &self,
        request_type: &RequestType,
    ) -> Option<&Result<Arc<Expr<Option<Type>>>, StaticError>> {
        self.envs.get(request_type).map(|d| &d.typed)
    }

    /// The possible outcomes of the condition in `request_type` (always
    /// sound); `None` for an environment not of the schema.
    pub fn outcomes(&self, request_type: &RequestType) -> Option<&NESet<EvaluationOutcome>> {
        self.envs.get(request_type).map(|d| &d.outcomes)
    }

    /// The condition as a residual in `request_type`; `None` for an
    /// environment not of the schema.
    #[cfg(feature = "tpe")]
    pub fn residual(&self, request_type: &RequestType) -> Option<&Arc<Residual>> {
        self.envs.get(request_type).map(|d| &d.residual)
    }

    /// Whether `schema` is the schema the conditions were typechecked against.
    pub fn same_schema(&self, schema: &ValidatorSchema) -> bool {
        std::ptr::eq(schema, &*self.schema) || *schema == *self.schema
    }

    /// Replaces what is known in `request_type` by the result of an analysis
    /// that produced `typed` (semantically equivalent to the condition) and
    /// `outcomes` (sound for it) — the symbolic evaluator's fold.
    ///
    /// Fails if `request_type` is not an environment of the schema or `typed`
    /// has a node without a type; with `tpe`, the residual is rebuilt from
    /// `typed`.
    pub fn fold_env(
        &mut self,
        request_type: &RequestType,
        typed: Arc<Expr<Option<Type>>>,
        outcomes: NESet<EvaluationOutcome>,
    ) -> Result<(), TypecheckError> {
        let data = self
            .envs
            .get_mut(request_type)
            .ok_or(TypecheckError::MissingType)?;
        #[cfg(feature = "tpe")]
        {
            data.residual = Arc::new(
                Residual::try_from_typed_expr(&typed, &SlotEnv::new())
                    .map_err(|_| TypecheckError::MissingType)?,
            );
        }
        data.typed = Ok(typed);
        data.outcomes = outcomes;
        Ok(())
    }

    /// Interprets every environment's residual against the maximally unknown
    /// request of that environment (known principal, action and resource
    /// types; unknown entities and context) and records the result.
    #[cfg(feature = "tpe")]
    fn constant_fold(&mut self) {
        use crate::extensions::Extensions;
        use crate::tpe::entities::PartialEntities;
        use crate::tpe::evaluator::Evaluator;
        use crate::tpe::request::{PartialEntityUID, PartialRequest};

        let entities = PartialEntities::new();
        for (request_type, data) in &mut self.envs {
            // Cannot fail for an environment the schema enumerates; an
            // environment it does fail for is left as it is.
            let Ok(request) = PartialRequest::new(
                PartialEntityUID {
                    ty: request_type.principal.clone(),
                    eid: None,
                },
                request_type.action.clone(),
                PartialEntityUID {
                    ty: request_type.resource.clone(),
                    eid: None,
                },
                None,
                &self.schema,
            ) else {
                continue;
            };
            let evaluator = Evaluator {
                request: &request,
                entities: &entities,
                extensions: Extensions::all_available(),
            };
            let residual = evaluator.interpret(&data.residual);
            data.fold_residual(residual);
        }
    }
}

impl Typecheckable for TypecheckedPolicy {
    fn untyped(&self) -> &Expr {
        &self.condition
    }

    /// The cached typed condition when `schema` is the one the policy was
    /// typechecked against and `env` is one of its environments with a typed
    /// condition; otherwise the default (which is correct on any schema — a
    /// static-error environment's condition still always errors — just not
    /// free).
    fn typechecked(
        &self,
        env: &RequestEnv<'_>,
        schema: &ValidatorSchema,
    ) -> Result<Arc<Expr<Option<Type>>>, TypecheckError> {
        if self.same_schema(schema) {
            if let Some(Ok(typed)) = env
                .to_request_type()
                .and_then(|rt| self.typed_condition(&rt))
            {
                return Ok(typed.clone());
            }
        }
        typecheck_expr(&self.condition, env, schema).map(Arc::new)
    }
}

/// A policy set validated against a schema, with every executable policy
/// typechecked in every request environment.
#[derive(Debug, Clone)]
pub struct TypecheckedPolicySet {
    policies: PolicySet,
    schema: Arc<ValidatorSchema>,
    typechecked: LinkedHashMap<PolicyID, TypecheckedPolicy>,
}

impl TypecheckedPolicySet {
    /// Validates `policies` against `schema` (strict mode, as the validator
    /// does) and typechecks every executable policy — static or linked; a
    /// template is validated but has no condition to type — in every request
    /// environment.
    pub fn new(policies: PolicySet, schema: Arc<ValidatorSchema>) -> Result<Self, TypecheckError> {
        let result = Validator::new((*schema).clone()).validate(&policies, ValidationMode::Strict);
        if !result.validation_passed() {
            return Err(TypecheckError::NotWellTyped {
                errs: result.validation_errors().cloned().collect(),
            });
        }
        let mut typechecked = LinkedHashMap::new();
        for policy in policies.policies() {
            typechecked.insert(
                policy.id().clone(),
                TypecheckedPolicy::new(Arc::new(policy.clone()), Arc::clone(&schema))?,
            );
        }
        Ok(Self {
            policies,
            schema,
            typechecked,
        })
    }

    /// The policy set.
    pub fn policy_set(&self) -> &PolicySet {
        &self.policies
    }

    /// The policy set, giving up what was computed for it.
    pub fn into_policy_set(self) -> PolicySet {
        self.policies
    }

    /// The schema.
    pub fn schema(&self) -> &Arc<ValidatorSchema> {
        &self.schema
    }

    /// The typechecked policies, in the policy set's order.
    pub fn policies(&self) -> impl Iterator<Item = &TypecheckedPolicy> {
        self.typechecked.values()
    }

    /// The typechecked policies, mutably (for a fold).
    pub fn policies_mut(&mut self) -> impl Iterator<Item = &mut TypecheckedPolicy> {
        self.typechecked.iter_mut().map(|(_, p)| p)
    }

    /// The typechecked policy with `id`.
    pub fn get(&self, id: &PolicyID) -> Option<&TypecheckedPolicy> {
        self.typechecked.get(id)
    }

    /// The typechecked policy with `id`, mutably (for a fold).
    pub fn get_mut(&mut self, id: &PolicyID) -> Option<&mut TypecheckedPolicy> {
        self.typechecked.get_mut(id)
    }

    /// Constant-folds every policy's condition in every environment with
    /// type-aware partial evaluation against the maximally unknown request of
    /// that environment: scope constraints become literals, literal
    /// subexpressions are evaluated, and the outcomes narrow accordingly. A
    /// condition that always errors becomes a [`StaticError`].
    #[cfg(feature = "tpe")]
    pub fn constant_fold(mut self) -> Self {
        for (_, policy) in self.typechecked.iter_mut() {
            policy.constant_fold();
        }
        self
    }
}

/// Replaces every slot of `expr` by the entity it is bound to in `env`,
/// keeping the data of every node.
pub fn substitute_slots<T: Clone + Default>(
    expr: &Expr<T>,
    env: &SlotEnv,
) -> Result<Expr<T>, TypecheckError> {
    if expr.slots().next().is_none() {
        return Ok(expr.clone());
    }
    let rebuild = |e: &Arc<Expr<T>>| substitute_slots(e, env).map(Arc::new);
    let kind = match expr.expr_kind() {
        ExprKind::Slot(slot) => match env.get(slot) {
            Some(euid) => ExprKind::Lit(Literal::EntityUID(Arc::new(euid.clone()))),
            None => return Err(TypecheckError::UnboundSlot(*slot)),
        },
        ExprKind::Lit(_) | ExprKind::Var(_) | ExprKind::Unknown(_) => {
            return Ok(expr.clone());
        }
        #[cfg(feature = "tolerant-ast")]
        ExprKind::Error { .. } => return Ok(expr.clone()),
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => ExprKind::If {
            test_expr: rebuild(test_expr)?,
            then_expr: rebuild(then_expr)?,
            else_expr: rebuild(else_expr)?,
        },
        ExprKind::And { left, right } => ExprKind::And {
            left: rebuild(left)?,
            right: rebuild(right)?,
        },
        ExprKind::Or { left, right } => ExprKind::Or {
            left: rebuild(left)?,
            right: rebuild(right)?,
        },
        ExprKind::UnaryApp { op, arg } => ExprKind::UnaryApp {
            op: *op,
            arg: rebuild(arg)?,
        },
        ExprKind::BinaryApp { op, arg1, arg2 } => ExprKind::BinaryApp {
            op: *op,
            arg1: rebuild(arg1)?,
            arg2: rebuild(arg2)?,
        },
        ExprKind::ExtensionFunctionApp { fn_name, args } => ExprKind::ExtensionFunctionApp {
            fn_name: fn_name.clone(),
            args: Arc::new(
                args.iter()
                    .map(|a| substitute_slots(a, env))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        },
        ExprKind::GetAttr { expr, attr } => ExprKind::GetAttr {
            expr: rebuild(expr)?,
            attr: attr.clone(),
        },
        ExprKind::HasAttr { expr, attr } => ExprKind::HasAttr {
            expr: rebuild(expr)?,
            attr: attr.clone(),
        },
        ExprKind::Like { expr, pattern } => ExprKind::Like {
            expr: rebuild(expr)?,
            pattern: pattern.clone(),
        },
        ExprKind::Is { expr, entity_type } => ExprKind::Is {
            expr: rebuild(expr)?,
            entity_type: entity_type.clone(),
        },
        ExprKind::Set(elements) => ExprKind::Set(Arc::new(
            elements
                .iter()
                .map(|e| substitute_slots(e, env))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ExprKind::Record(map) => ExprKind::Record(Arc::new(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), substitute_slots(v, env)?)))
                .collect::<Result<_, TypecheckError>>()?,
        )),
    };
    Ok(ExprBuilder::with_data(expr.data().clone())
        .with_same_source_loc(expr)
        .with_expr_kind(kind))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ast::{EntityType, EntityUID, PolicyID};
    use crate::parser::{parse_policy_or_template, parse_policyset};
    use crate::validator::ValidatorSchema;
    use cool_asserts::assert_matches;
    use std::str::FromStr;

    const SCHEMA: &str = r#"
        entity User { age: Long };
        entity Group;
        entity Doc { owner: User, public: Bool };
        action view appliesTo { principal: [User, Group], resource: Doc };
        action edit appliesTo { principal: User, resource: Doc };
    "#;

    fn schema() -> Arc<ValidatorSchema> {
        Arc::new(ValidatorSchema::from_str(SCHEMA).expect("schema"))
    }

    fn request_type(principal: &str, action: &str, resource: &str) -> RequestType {
        RequestType {
            principal: EntityType::from_str(principal).expect("type"),
            action: EntityUID::from_str(action).expect("uid"),
            resource: EntityType::from_str(resource).expect("type"),
        }
    }

    fn policies(text: &str) -> PolicySet {
        parse_policyset(text).expect("policies")
    }

    fn all_request_types() -> Vec<RequestType> {
        vec![
            request_type("User", r#"Action::"view""#, "Doc"),
            request_type("Group", r#"Action::"view""#, "Doc"),
            request_type("User", r#"Action::"edit""#, "Doc"),
        ]
    }

    #[test]
    fn every_policy_in_every_env() {
        let ps = policies(
            r#"
            permit(principal, action, resource) when { resource.public };
            forbid(principal is User, action == Action::"edit", resource) when { principal.age < 18 };
            "#,
        );
        let tps = TypecheckedPolicySet::new(ps, schema()).expect("typechecks");
        assert_eq!(tps.policies().count(), 2);
        for policy in tps.policies() {
            let mut rts: Vec<_> = policy.request_types().cloned().collect();
            let mut expected = all_request_types();
            rts.sort_by_key(|rt| format!("{rt:?}"));
            expected.sort_by_key(|rt| format!("{rt:?}"));
            assert_eq!(rts, expected);
            for rt in &expected {
                let typed = policy.typed_condition(rt).expect("env");
                let typed = typed.as_ref().expect("no static error");
                assert!(typed.data().is_some(), "root typed");
                assert!(typed.slots().next().is_none());
                // Nothing was folded: the permit (which applies everywhere)
                // can still go either way; the forbid's scope excludes some
                // environments, where the typechecker already knows `false`.
                let outcomes = policy.outcomes(rt).expect("env");
                assert!(outcomes.contains(&EvaluationOutcome::False));
                if policy.policy().effect() == Effect::Permit {
                    assert!(outcomes.contains(&EvaluationOutcome::True));
                }
                #[cfg(not(feature = "tpe"))]
                assert_eq!(outcomes, &all_outcomes());
            }
        }
    }

    #[test]
    fn ill_typed_policy_is_rejected() {
        let ps = policies(r#"permit(principal, action, resource) when { resource.nope };"#);
        assert_matches!(
            TypecheckedPolicySet::new(ps, schema()),
            Err(TypecheckError::NotWellTyped { errs }) => assert!(!errs.is_empty())
        );
    }

    #[test]
    fn linked_policy_has_its_slots_substituted() {
        let mut ps = PolicySet::new();
        let template = parse_policy_or_template(
            Some(PolicyID::from_string("t")),
            r#"permit(principal == ?principal, action, resource);"#,
        )
        .expect("template");
        ps.add_template(template).expect("add");
        let alice = EntityUID::from_str(r#"User::"alice""#).expect("uid");
        ps.link(
            PolicyID::from_string("t"),
            PolicyID::from_string("l"),
            HashMap::from([(SlotId::principal(), alice.clone())]),
        )
        .expect("link");
        let tps = TypecheckedPolicySet::new(ps, schema()).expect("typechecks");
        let linked = tps.get(&PolicyID::from_string("l")).expect("link present");
        assert!(
            tps.get(&PolicyID::from_string("t")).is_none(),
            "templates carry no condition"
        );
        assert!(linked.condition().slots().next().is_none());
        assert!(format!("{}", linked.condition()).contains(r#"User::"alice""#));
        for rt in all_request_types() {
            let typed = linked
                .typed_condition(&rt)
                .expect("env")
                .as_ref()
                .expect("typed");
            assert!(typed.slots().next().is_none());
        }
    }

    #[test]
    fn trait_answers_from_the_cache_and_falls_back() {
        let ps = policies(r#"permit(principal, action, resource) when { resource.public };"#);
        let schema = schema();
        let tps = TypecheckedPolicySet::new(ps, Arc::clone(&schema)).expect("typechecks");
        let policy = tps.policies().next().expect("one");
        let rt = request_type("User", r#"Action::"view""#, "Doc");
        let env = schema
            .unlinked_request_envs(ValidationMode::Strict)
            .find(|e| e.to_request_type().as_ref() == Some(&rt))
            .expect("env");
        let cached = policy
            .typed_condition(&rt)
            .expect("env")
            .as_ref()
            .expect("typed");
        let answer = policy.typechecked(&env, &schema).expect("typechecks");
        assert!(
            Arc::ptr_eq(cached, &answer),
            "the cached Arc is returned as is"
        );
        // A structurally equal schema is the same schema.
        let other = ValidatorSchema::from_str(SCHEMA).expect("schema");
        let answer = policy.typechecked(&env, &other).expect("typechecks");
        assert!(Arc::ptr_eq(cached, &answer));
        // A different schema takes the default path: a fresh, equivalent typing.
        let bigger = ValidatorSchema::from_str(&format!("{SCHEMA} entity Other;")).expect("schema");
        let bigger_env = bigger
            .unlinked_request_envs(ValidationMode::Strict)
            .find(|e| e.to_request_type().as_ref() == Some(&rt))
            .expect("env");
        let answer = policy
            .typechecked(&bigger_env, &bigger)
            .expect("typechecks");
        assert!(!Arc::ptr_eq(cached, &answer));
        assert!(answer.data().is_some());
        // The plain expression path is the default.
        let plain = policy
            .condition()
            .typechecked(&env, &schema)
            .expect("typechecks");
        assert!(plain.data().is_some());
    }

    #[cfg(feature = "tpe")]
    mod tpe {
        use super::*;

        #[test]
        fn construction_converts_without_interpreting() {
            let ps = policies(r#"permit(principal, action == Action::"edit", resource);"#);
            let tps = TypecheckedPolicySet::new(ps, schema()).expect("typechecks");
            let policy = tps.policies().next().expect("one");
            // Not folded: the action constraint is still a comparison, so the
            // outcomes are those of the typed expression alone.
            let view = request_type("User", r#"Action::"view""#, "Doc");
            assert!(policy.residual(&view).expect("env").is_partial());
        }

        #[test]
        fn constant_fold_decides_scope_per_env_and_narrows_outcomes() {
            let ps = policies(
                r#"permit(principal, action == Action::"edit", resource) when { resource.public };"#,
            );
            let tps = TypecheckedPolicySet::new(ps, schema())
                .expect("typechecks")
                .constant_fold();
            let policy = tps.policies().next().expect("one");
            let view = request_type("User", r#"Action::"view""#, "Doc");
            let edit = request_type("User", r#"Action::"edit""#, "Doc");
            // In a `view` environment the policy is `false`.
            assert!(policy.residual(&view).expect("env").is_false());
            assert_eq!(
                policy.outcomes(&view).expect("env"),
                &nes![EvaluationOutcome::False]
            );
            let typed = policy
                .typed_condition(&view)
                .expect("env")
                .as_ref()
                .expect("typed");
            assert_matches!(typed.expr_kind(), ExprKind::Lit(Literal::Bool(false)));
            // In an `edit` environment only the `when` clause is left, with its
            // type; `resource.public` can go either way (and may error: the
            // entity is unknown, so it may be missing from the store).
            let typed = policy
                .typed_condition(&edit)
                .expect("env")
                .as_ref()
                .expect("typed");
            assert_matches!(typed.expr_kind(), ExprKind::GetAttr { .. });
            assert_eq!(typed.data(), &Some(Type::primitive_boolean()));
            let outcomes = policy.outcomes(&edit).expect("env");
            assert!(outcomes.contains(&EvaluationOutcome::True));
            assert!(outcomes.contains(&EvaluationOutcome::False));
        }

        #[test]
        fn constant_fold_static_error() {
            let ps = policies(
                r#"permit(principal, action, resource) when { 9223372036854775807 + 1 == 1 };"#,
            );
            let tps = TypecheckedPolicySet::new(ps, schema())
                .expect("typechecks")
                .constant_fold();
            let policy = tps.policies().next().expect("one");
            let rt = request_type("User", r#"Action::"view""#, "Doc");
            assert_matches!(
                policy.typed_condition(&rt).expect("env"),
                Err(StaticError { ty }) => assert_eq!(ty, &Type::primitive_boolean())
            );
            assert_eq!(
                policy.outcomes(&rt).expect("env"),
                &nes![EvaluationOutcome::Error]
            );
        }

        #[test]
        fn constant_fold_nested_error_keeps_the_condition() {
            let ps = policies(
                r#"permit(principal, action, resource) when { resource.public || 9223372036854775807 + 1 == 1 };"#,
            );
            let tps = TypecheckedPolicySet::new(ps, schema())
                .expect("typechecks")
                .constant_fold();
            let policy = tps.policies().next().expect("one");
            let rt = request_type("User", r#"Action::"view""#, "Doc");
            let residual = policy.residual(&rt).expect("env");
            assert!(residual.is_partial());
            // The typed condition is the unfolded one (the scope conjuncts
            // are still there), since the residual has an error node.
            let typed = policy
                .typed_condition(&rt)
                .expect("env")
                .as_ref()
                .expect("typed");
            assert_matches!(typed.expr_kind(), ExprKind::And { .. });
            assert_eq!(
                policy.outcomes(&rt).expect("env"),
                &nes![EvaluationOutcome::True, EvaluationOutcome::Error]
            );
        }
    }
}
