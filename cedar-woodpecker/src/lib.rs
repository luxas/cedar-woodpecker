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

//! Woodpecker finds privilege-escalation paths in a Cedar policy set and
//! synthesizes the policies they imply.
//!
//! A principal's explicit permissions are the cubes of the policy set's
//! allow-only DNF that the symbolic evaluator cannot rule out in a request
//! environment ([`source_cubes`]). A *transition function* says which
//! permissions in which environments give an implicit permission in a target
//! environment ([`Transition`]): the source cubes are combined with it in a
//! copy of the schema whose target action's context carries the intermediate
//! request (`context.action{i}`, `context.resource{i}`), split into cubes
//! again, and pruned with the evaluator; every surviving cube is an
//! escalation path, from which a policy over the target environment is
//! synthesized by eliminating the intermediate request ([`escalate`]).
//!
//! The elimination ([`escalate::eliminate`]) first propagates what the path's
//! equalities say — constants, then the canonical members of each equivalence
//! class ([`equiv::rewrite`]) — then rewrites pairs of set operations over
//! the same intermediate set or element, and pairs of `in`s over the same
//! intermediate entity, into one operation over what remains
//! ([`sets::rewrite`], plan 13's matrix), and finally drops every conjunct
//! that still mentions the intermediate request. The long bounds are the
//! refinement to come.
//!
//! The name: Levaillant's woodpecker lives in the cedar forests of North
//! Africa, and a cedar is a dense tree with good defenses — yet the woodpecker
//! punches holes in it.

#![warn(missing_docs)]

pub mod cubes;
pub mod equiv;
pub mod escalate;
pub mod expr;
pub mod schema;
pub mod sets;
pub mod transition;

use cedar_policy::{Entities, PolicySet, RequestEnv, Schema, SchemaFragment, TypecheckError};
use cedar_policy_core::ast::Expr;
use cedar_policy_symcc::dnf::{DnfError, DEFAULT_MAX_CUBES, DEFAULT_MAX_SPLIT_NODES};
use cedar_policy_symcc::evaluator::{EvaluationError, Evaluator};
use cedar_policy_symcc::semantics::{collect_semantics, Assumption};
use cedar_policy_symcc::solver::Solver;
use cedar_policy_symcc::CedarSymCompiler;

pub use cubes::{source_cubes, SourceCube};
pub use escalate::{escalate, Escalation};
pub use schema::{extend_schema, ExtendedSchema};
pub use transition::{SourceEnv, Transition};

/// Everything that can go wrong.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    /// The schema does not parse.
    #[error("schema: {0}")]
    Schema(String),
    /// The policies do not parse.
    #[error("policies: {0}")]
    Policies(String),
    /// The transition file does not parse, or a transition is not well formed.
    #[error("transitions: {0}")]
    Transitions(String),
    /// The entities do not parse.
    #[error("entities: {0}")]
    Entities(String),
    /// A `@semantics` assumption is not well typed in some request environment.
    #[error("the @semantics assumption on {origin} (`{expression}`) is not well typed in the request environment {env}: {message}")]
    AssumptionNotWellTyped {
        /// The declaration the annotation is on.
        origin: String,
        /// The assumption as written.
        expression: String,
        /// The environment.
        env: String,
        /// The typechecker's message.
        message: String,
    },
    /// The assumptions contradict each other (or the schema) in some request
    /// environment.
    #[error("the assumptions are unsatisfiable in the request environment {env}")]
    Unsatisfiable {
        /// The environment.
        env: String,
    },
    /// The policies (or a synthesized policy) do not validate.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Typecheck(#[from] TypecheckError),
    /// The DNF conversion failed.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Dnf(#[from] DnfError),
    /// The symbolic evaluator failed.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Evaluation(#[from] EvaluationError),
    /// SymCC failed.
    #[error(transparent)]
    #[diagnostic(transparent)]
    SymCC(#[from] cedar_policy_symcc::err::Error),
    /// A bug.
    #[error("internal: {0}")]
    Internal(String),
}

/// The DNF budgets of a run.
#[derive(Debug, Clone, Copy)]
pub struct Budgets {
    /// The node budget of an atom split (`dnf::DEFAULT_MAX_SPLIT_NODES`).
    pub max_split_nodes: usize,
    /// The cube budget of a policy (`dnf::DEFAULT_MAX_CUBES`).
    pub max_cubes: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_split_nodes: DEFAULT_MAX_SPLIT_NODES,
            max_cubes: DEFAULT_MAX_CUBES,
        }
    }
}

/// A schema in the three forms the pipeline needs: validated, as its JSON
/// fragment (the form the extended copies are made from), and with its
/// `@semantics` assumptions.
#[derive(Debug, Clone)]
pub struct LoadedSchema {
    /// The schema.
    pub schema: Schema,
    /// The schema as a JSON schema fragment.
    pub json: serde_json::Value,
    /// The `@semantics` assumptions declared in the schema.
    pub assumptions: Vec<Assumption>,
}

impl LoadedSchema {
    /// Parses a schema from text: the JSON schema format when `json`, else the
    /// Cedar schema syntax.
    pub fn parse(text: &str, json: bool) -> Result<Self, Error> {
        let (schema, value) = if json {
            let schema = Schema::from_json_str(text).map_err(|e| Error::Schema(e.to_string()))?;
            let value: serde_json::Value =
                serde_json::from_str(text).map_err(|e| Error::Schema(e.to_string()))?;
            (schema, value)
        } else {
            let (schema, _) =
                Schema::from_cedarschema_str(text).map_err(|e| Error::Schema(e.to_string()))?;
            let (fragment, _) = SchemaFragment::from_cedarschema_str(text)
                .map_err(|e| Error::Schema(e.to_string()))?;
            let value = fragment
                .to_json_value()
                .map_err(|e| Error::Schema(e.to_string()))?;
            (schema, value)
        };
        let assumptions =
            collect_semantics(text, json, &schema).map_err(|e| Error::Schema(e.to_string()))?;
        Ok(Self {
            schema,
            json: value,
            assumptions,
        })
    }

    /// The assumptions as expressions.
    pub fn assumption_exprs(&self) -> Vec<Expr> {
        self.assumptions
            .iter()
            .map(|a| a.expression.clone())
            .collect()
    }
}

/// Parses a policy set from Cedar text.
pub fn parse_policies(text: &str) -> Result<PolicySet, Error> {
    text.parse()
        .map_err(|e: cedar_policy::ParseErrors| Error::Policies(e.to_string()))
}

/// Parses entities from their JSON form, against `schema`.
pub fn parse_entities(text: &str, schema: &Schema) -> Result<Entities, Error> {
    Entities::from_json_str(text, Some(schema)).map_err(|e| Error::Entities(e.to_string()))
}

/// The three names of a request environment, for messages.
pub fn env_name(env: &RequestEnv) -> String {
    format!(
        "(principal: {}, action: {}, resource: {})",
        env.principal(),
        env.action(),
        env.resource()
    )
}

/// An evaluator on `schema` with the assumptions and entities registered:
/// every assumption is typechecked in every environment first, and the
/// assumptions are checked satisfiable in every environment (one solver
/// query each) so that nothing downstream is vacuous.
pub async fn evaluator_with<S: Solver + Send>(
    compiler: CedarSymCompiler<S>,
    schema: &Schema,
    assumptions: &[Assumption],
    entities: Option<&Entities>,
) -> Result<Evaluator<S>, Error> {
    let mut evaluator = Evaluator::new(compiler, schema)?;
    let envs: Vec<_> = schema.request_envs().collect();
    for env in &envs {
        for assumption in assumptions {
            if let Err(e) = evaluator.typecheck_assumption(&assumption.expression, env) {
                return Err(Error::AssumptionNotWellTyped {
                    origin: assumption.origin.clone(),
                    expression: assumption.source.clone(),
                    env: env_name(env),
                    message: match e {
                        EvaluationError::AssumptionNotWellTyped { errs, .. }
                        | EvaluationError::NotWellTyped { errs } => errs
                            .iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join("; "),
                        other => other.to_string(),
                    },
                });
            }
        }
    }
    for assumption in assumptions {
        evaluator.assume_expr(assumption.expression.clone());
    }
    if let Some(entities) = entities {
        evaluator.assume_entities(entities)?;
    }
    if !assumptions.is_empty() || entities.is_some() {
        for env in &envs {
            match evaluator
                .evaluate(&Expr::val(true), env, std::iter::empty())
                .await
            {
                Ok(_) => {}
                Err(EvaluationError::UnsatisfiableAssumptions) => {
                    return Err(Error::Unsatisfiable { env: env_name(env) })
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(evaluator)
}
