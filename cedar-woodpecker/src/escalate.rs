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

//! Escalation paths and the policies they imply.
//!
//! For a transition and a principal type, the source cubes of every source
//! environment are renamed to the intermediate request they become in the
//! target environment (`action` ↦ `context.action{i}`, `resource` ↦
//! `context.resource{i}`, `context` ↦ `context.context{i}`), disjoined per
//! source, conjoined over the sources
//! and with the transition's condition, and the result — a permit over the
//! target environment of the extended schema — is split into cubes and
//! folded with the symbolic evaluator. Every cube that can be `true` is a
//! path. From a path, [`eliminate`] synthesizes the implied policy over the
//! original schema, and the path itself, assumed, checks that policy sound.

use std::sync::Arc;

use cedar_policy::{
    Entities, EntityTypeName, EntityUid, EvaluationOutcome, Policy, PolicyId, PolicySet,
    RequestEnv, Schema, TypecheckError,
};
use cedar_policy_core::ast::{
    self, ActionConstraint, Annotation, Annotations, AnyId, Effect, Expr, Literal, PolicyID,
    PrincipalConstraint, ResourceConstraint, StaticPolicy,
};
use cedar_policy_symcc::dnf::{allow_cubes, conjuncts, DnfError};
use cedar_policy_symcc::semantics::Assumption;
use cedar_policy_symcc::solver::Solver;
use cedar_policy_symcc::typechecked::SymbolicFold;
use cedar_policy_symcc::CedarSymCompiler;
use serde::Serialize;
use smol_str::SmolStr;

use crate::cubes::{erase, SourceCube};
use crate::expr::{and_chain, context_attr, mentions_context_attrs, or_chain, rename_vars};
use crate::schema::extend_schema;
use crate::transition::Transition;
use crate::{env_name, evaluator_with, Budgets, Error};

/// An escalation path and the policy it implies.
#[derive(Debug, Clone)]
pub struct Escalation {
    /// The transition's name.
    pub transition: String,
    /// The principal type.
    pub principal: EntityTypeName,
    /// Per source environment, the source cubes that were combined (the
    /// path's condition shows which of them survive).
    pub sources: Vec<(RequestEnv, Vec<PolicyId>)>,
    /// The target environment.
    pub target: RequestEnv,
    /// The path cube's id.
    pub path_id: PolicyId,
    /// The path: the cube's condition over the target environment of the
    /// extended schema, folded.
    pub path: Expr,
    /// The implied policy, over the original schema.
    pub policy: Policy,
    /// Whether the path implies the policy (the must-have check); `false`
    /// when there is a `problem`.
    pub sound: bool,
    /// Why the policy could not be checked: it does not validate against
    /// the original schema (a bug in the elimination, reported per path).
    pub problem: Option<String>,
}

impl Escalation {
    /// The escalation as JSON.
    pub fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct Source {
            principal: String,
            action: String,
            resource: String,
            cubes: Vec<String>,
        }
        #[derive(Serialize)]
        struct Json {
            transition: String,
            principal: String,
            sources: Vec<Source>,
            target: Source,
            path_id: String,
            path: String,
            policy: String,
            sound: bool,
            problem: Option<String>,
        }
        let source = |env: &RequestEnv, cubes: &[PolicyId]| Source {
            principal: env.principal().to_string(),
            action: env.action().to_string(),
            resource: env.resource().to_string(),
            cubes: cubes.iter().map(ToString::to_string).collect(),
        };
        serde_json::to_value(Json {
            transition: self.transition.clone(),
            principal: self.principal.to_string(),
            sources: self
                .sources
                .iter()
                .map(|(env, cubes)| source(env, cubes))
                .collect(),
            target: source(&self.target, std::slice::from_ref(&self.path_id)),
            path_id: self.path_id.to_string(),
            path: self.path.to_string(),
            policy: self.policy.to_cedar().unwrap_or_default(),
            sound: self.sound,
            problem: self.problem.clone(),
        })
        .unwrap_or(serde_json::Value::Null)
    }

    /// The escalation as text: a provenance comment and the policy.
    pub fn to_text(&self) -> String {
        let sources = self
            .sources
            .iter()
            .map(|(env, cubes)| {
                format!(
                    "{} via {}",
                    env_name(env),
                    cubes
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "// {}: {} -> {} [{}]\n// path: {}\n{}\n",
            self.transition,
            sources,
            env_name(&self.target),
            match &self.problem {
                Some(problem) => format!("NOT VALID: {problem}"),
                None if self.sound => "sound".to_string(),
                None => "NOT SOUND".to_string(),
            },
            self.path,
            self.policy.to_cedar().unwrap_or_default()
        )
    }
}

/// What every transition is run against.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'a> {
    /// The original schema.
    pub schema: &'a Schema,
    /// Its JSON form (the extended copies are made from it).
    pub schema_json: &'a serde_json::Value,
    /// The `@semantics` assumptions.
    pub assumptions: &'a [Assumption],
    /// Entity data to assume.
    pub entities: Option<&'a Entities>,
    /// The DNF budgets.
    pub budgets: Budgets,
}

/// The elimination: the path's equalities are propagated first
/// ([`crate::equiv::rewrite`] — constants, then the canonical member of each
/// equivalence class), pairs of set operations and `in`s over the same
/// intermediate term become one operation over the rest
/// ([`crate::sets::rewrite`], then the classes once more for what that
/// produced), and every conjunct that still mentions the intermediate
/// request (an attribute in `added`) becomes `true`; the rest is the implied
/// policy's condition (`None` when nothing is left), the `has`/`hasTag`
/// guards first — a canonical substituted into an earlier conjunct may have
/// moved an access before the guard the transition supplied, and the
/// validator reads `&&` left to right — then the others in their order,
/// equalities oriented with the constant on the right. Sound: every rewrite
/// keeps a conjunction implied by the path, dropping conjuncts weakens, and
/// `&&` over error-free conjuncts commutes (a guard errs on nothing).
pub fn eliminate(path: &Expr, added: &[SmolStr]) -> Option<Expr> {
    let conjuncts: Vec<Expr> = conjuncts(path).into_iter().cloned().collect();
    let rewritten = crate::equiv::rewrite(&conjuncts, added);
    let rewritten = crate::sets::rewrite(&rewritten, added);
    let rewritten = crate::equiv::rewrite(&rewritten, added);
    let (guards, rest): (Vec<Expr>, Vec<Expr>) = rewritten
        .into_iter()
        .filter(|c| !mentions_context_attrs(c, added))
        .partition(is_guard);
    and_chain(guards.into_iter().chain(rest).map(|c| orient(&c)))
}

/// A positive `has`/`hasTag`: what gives the validator a capability.
fn is_guard(e: &Expr) -> bool {
    matches!(
        e.expr_kind(),
        cedar_policy_core::ast::ExprKind::HasAttr { .. }
            | cedar_policy_core::ast::ExprKind::BinaryApp {
                op: cedar_policy_core::ast::BinaryOp::HasTag,
                ..
            }
    )
}

/// `lit == x` as `x == lit`.
fn orient(e: &Expr) -> Expr {
    match e.expr_kind() {
        cedar_policy_core::ast::ExprKind::BinaryApp {
            op: cedar_policy_core::ast::BinaryOp::Eq,
            arg1,
            arg2,
        } if crate::expr::is_constant(arg1) && !crate::expr::is_constant(arg2) => {
            Expr::is_eq(arg2.as_ref().clone(), arg1.as_ref().clone())
        }
        _ => e.clone(),
    }
}

/// A typecheck failure with the validator's own messages.
fn typecheck_message(e: &TypecheckError) -> String {
    match e {
        TypecheckError::NotWellTyped { errs } if !errs.is_empty() => errs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
    }
}

/// The `@woodpecker("<text>")` annotation.
pub(crate) fn annotation(text: &str) -> Result<Annotations, Error> {
    let key: AnyId = "woodpecker"
        .parse()
        .map_err(|e: cedar_policy_core::parser::err::ParseErrors| Error::Internal(e.to_string()))?;
    Ok(Annotations::from_iter([(
        key,
        Annotation {
            val: text.into(),
            loc: None,
        },
    )]))
}

/// `permit(principal is pt, action == at, resource is rt) when { when }`.
pub(crate) fn permit(
    id: &str,
    annotations: Annotations,
    pt: &EntityTypeName,
    action: &EntityUid,
    rt: &EntityTypeName,
    when: Option<Expr>,
) -> Result<ast::Policy, Error> {
    StaticPolicy::new(
        PolicyID::from_string(id),
        None,
        annotations,
        Effect::Permit,
        PrincipalConstraint::is_entity_type(Arc::new(pt.as_ref().clone())),
        ActionConstraint::is_eq(action.as_ref().clone()),
        ResourceConstraint::is_entity_type(Arc::new(rt.as_ref().clone())),
        when,
    )
    .map(Into::into)
    .map_err(|e| Error::Internal(e.to_string()))
}

/// The escalation paths of `transitions` over `sources`, and the policies
/// they imply. `compiler` (one solver) is used for every transition's
/// evaluator and returned.
pub async fn escalate<S: Solver + Send>(
    sources: &[SourceCube],
    transitions: &[Transition],
    setup: Setup<'_>,
    compiler: CedarSymCompiler<S>,
) -> Result<(Vec<Escalation>, CedarSymCompiler<S>), Error> {
    let mut compiler = Some(compiler);
    let mut out = Vec::new();
    for transition in transitions {
        let extended = extend_schema(setup.schema_json, transition)?;
        for pt in transition.principal_types(setup.schema) {
            // the candidates of every source environment; none anywhere ⇒ no path
            let mut per_source: Vec<(RequestEnv, Vec<&SourceCube>)> = Vec::new();
            for source in &transition.sources {
                let env = source.env(&pt);
                let cubes: Vec<&SourceCube> = sources.iter().filter(|c| c.env == env).collect();
                per_source.push((env, cubes));
            }
            if per_source.iter().any(|(_, cubes)| cubes.is_empty()) {
                continue;
            }
            let target = transition.target.env(&pt);
            // the condition must typecheck on its own: through the combined
            // condition it would inherit the source cubes' `has` guards
            let when_policy = permit(
                &format!("woodpecker.{}.when", transition.name),
                Annotations::new(),
                &pt,
                &transition.target.action,
                &transition.target.resource,
                Some(transition.when.clone()),
            )?;
            PolicySet::from_policies([Policy::from(when_policy)])
                .map_err(|e| Error::Internal(e.to_string()))?
                .typecheck(&extended.schema)
                .map_err(|e| {
                    Error::Transitions(format!(
                        "transition `{}`: `when` does not typecheck in {}: {}",
                        transition.name,
                        env_name(&target),
                        typecheck_message(&e)
                    ))
                })?;
            // the combined condition over the target environment
            let mut conj: Vec<Expr> = Vec::new();
            for (attrs, (env, cubes)) in transition.context_attrs().into_iter().zip(&per_source) {
                let action_e = context_attr(&attrs.action);
                let resource_e = context_attr(&attrs.resource);
                let context_e = context_attr(&attrs.context);
                conj.push(Expr::is_eq(
                    action_e.clone(),
                    Expr::val(Literal::EntityUID(Arc::new(env.action().as_ref().clone()))),
                ));
                if let Some(disj) = or_chain(
                    cubes
                        .iter()
                        .map(|c| rename_vars(&c.condition, &action_e, &resource_e, &context_e)),
                ) {
                    conj.push(disj);
                }
            }
            conj.push(transition.when.clone());
            let when = and_chain(conj);
            let combined = permit(
                &format!("woodpecker.{}", transition.name),
                Annotations::new(),
                &pt,
                &transition.target.action,
                &transition.target.resource,
                when,
            )?;
            let mut set = ast::PolicySet::new();
            set.add(combined)
                .map_err(|e| Error::Internal(e.to_string()))?;
            let paths = allow_cubes(
                &set,
                &extended.schema,
                setup.budgets.max_split_nodes,
                setup.budgets.max_cubes,
            )
            .map_err(|e| match e {
                DnfError::NotWellTyped { errs } => Error::Transitions(format!(
                    "transition `{}` (principal {pt}) does not typecheck: {}",
                    transition.name,
                    errs.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
                other => other.into(),
            })?;
            let Some(c) = compiler.take() else {
                return Err(Error::Internal("the solver was lost".to_string()));
            };
            let mut evaluator =
                evaluator_with(c, &extended.schema, setup.assumptions, setup.entities).await?;
            let folded = PolicySet::from(paths)
                .typecheck(&extended.schema)?
                .constant_fold()
                .symbolic_fold(&mut evaluator)
                .await?;
            let source_ids: Vec<(RequestEnv, Vec<PolicyId>)> = per_source
                .iter()
                .map(|(env, cubes)| (env.clone(), cubes.iter().map(|c| c.id.clone()).collect()))
                .collect();
            for (k, cube) in folded.policies().enumerate() {
                let Some(outcomes) = cube.outcomes(&target) else {
                    continue;
                };
                if !outcomes.contains(&EvaluationOutcome::True) {
                    continue;
                }
                let Some(Ok(typed)) = cube.typed_condition(&target) else {
                    continue;
                };
                let path = erase(typed);
                let rest = eliminate(&path, &extended.added);
                let annotations = annotation(&format!(
                    "{}: {}",
                    transition.name,
                    source_ids
                        .iter()
                        .flat_map(|(_, ids)| ids.iter().map(ToString::to_string))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
                let policy = permit(
                    &format!("woodpecker.{}.{k}", transition.name),
                    annotations,
                    &pt,
                    &transition.target.action,
                    &transition.target.resource,
                    rest.clone(),
                )?;
                let policy = Policy::from(policy);
                // the implied policy must validate against the original
                // schema; when it does not, that is the finding for this path
                let problem = PolicySet::from_policies([policy.clone()])
                    .map_err(|e| Error::Internal(e.to_string()))?
                    .typecheck(setup.schema)
                    .err()
                    .map(|e| {
                        format!(
                            "the synthesized policy does not validate against the schema: {}",
                            typecheck_message(&e)
                        )
                    });
                let sound = match (&problem, &rest) {
                    (Some(_), _) => false,
                    (None, None) => true,
                    (None, Some(rest)) => {
                        let result = evaluator
                            .evaluate(rest, &target, std::iter::once(path.clone()))
                            .await?;
                        !result.data().outcomes().contains(&EvaluationOutcome::False)
                    }
                };
                out.push(Escalation {
                    transition: transition.name.clone(),
                    principal: pt.clone(),
                    sources: source_ids.clone(),
                    target: target.clone(),
                    path_id: cube.id(),
                    path,
                    policy,
                    sound,
                    problem,
                });
            }
            compiler = Some(evaluator.into_compiler());
        }
    }
    compiler
        .map(|c| (out, c))
        .ok_or_else(|| Error::Internal("the solver was lost".to_string()))
}
