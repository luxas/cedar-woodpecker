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

//! Source cubes: a principal's explicit permissions per request environment.
//!
//! The policy set becomes allow-only cubes (`dnf::allow_cubes`: the forbids
//! are folded into the permits as deny witnesses, and every permit is split
//! into the cubes of its DNF, ids `{pid}.cube{i}`), the cube set is
//! typechecked once in every environment, constant-folded with TPE (the
//! scope conjuncts become literals per environment) and folded with the
//! symbolic evaluator under its assumptions; every cube that can still be
//! `true` in an environment is a source cube there.

use cedar_policy::{EvaluationOutcome, PolicyId, PolicySet, RequestEnv, Schema};
use cedar_policy_core::ast::{Expr, ExprBuilder};
use cedar_policy_core::validator::types::Type;
use cedar_policy_symcc::dnf::allow_cubes;
use cedar_policy_symcc::evaluator::Evaluator;
use cedar_policy_symcc::solver::Solver;
use cedar_policy_symcc::typechecked::SymbolicFold;
use nonempty_collections::NESet;
use serde::Serialize;

use crate::{env_name, Budgets, Error};

/// A cube of the allow-only DNF that can be `true` in a request environment.
#[derive(Debug, Clone)]
pub struct SourceCube {
    /// The environment.
    pub env: RequestEnv,
    /// The cube policy's id (`{pid}.cube{i}`).
    pub id: PolicyId,
    /// The cube's condition, folded in the environment (types erased): the
    /// scope conjuncts are gone, and what the assumptions decide is folded.
    pub condition: Expr,
    /// Its possible outcomes in the environment (`True` among them).
    pub outcomes: NESet<EvaluationOutcome>,
}

impl SourceCube {
    /// The cube as JSON.
    pub fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct Json<'a> {
            principal: String,
            action: String,
            resource: String,
            id: String,
            condition: String,
            outcomes: Vec<&'a str>,
        }
        serde_json::to_value(Json {
            principal: self.env.principal().to_string(),
            action: self.env.action().to_string(),
            resource: self.env.resource().to_string(),
            id: self.id.to_string(),
            condition: self.condition.to_string(),
            outcomes: outcome_names(&self.outcomes),
        })
        .unwrap_or(serde_json::Value::Null)
    }
}

/// The names of the outcomes, in a fixed order.
pub fn outcome_names(outcomes: &NESet<EvaluationOutcome>) -> Vec<&'static str> {
    [
        (EvaluationOutcome::True, "True"),
        (EvaluationOutcome::False, "False"),
        (EvaluationOutcome::Error, "Error"),
    ]
    .into_iter()
    .filter(|(o, _)| outcomes.contains(o))
    .map(|(_, n)| n)
    .collect()
}

/// A typed expression without its types (structure preserved, two boolean
/// literals under `&&`/`||` folded).
pub fn erase(typed: &Expr<Option<Type>>) -> Expr {
    typed.clone().into_expr::<ExprBuilder<()>>()
}

/// The source cubes of `policies` under `evaluator`'s assumptions, ordered
/// by environment (principal, action, resource) then cube id.
pub async fn source_cubes<S: Solver + Send>(
    policies: &PolicySet,
    schema: &Schema,
    evaluator: &mut Evaluator<S>,
    budgets: Budgets,
) -> Result<Vec<SourceCube>, Error> {
    let cubes = allow_cubes(
        policies.as_ref(),
        schema,
        budgets.max_split_nodes,
        budgets.max_cubes,
    )?;
    let cubes = PolicySet::from(cubes);
    let folded = cubes
        .typecheck(schema)?
        .constant_fold()
        .symbolic_fold(evaluator)
        .await?;
    let mut out = Vec::new();
    for env in schema.request_envs() {
        for policy in folded.policies() {
            let Some(outcomes) = policy.outcomes(&env) else {
                continue;
            };
            if !outcomes.contains(&EvaluationOutcome::True) {
                continue;
            }
            let Some(Ok(typed)) = policy.typed_condition(&env) else {
                continue;
            };
            out.push(SourceCube {
                env: env.clone(),
                id: policy.id(),
                condition: erase(typed),
                outcomes: outcomes.clone(),
            });
        }
    }
    // a stable order: the schema's environment order is not one
    out.sort_by_key(|c| {
        (
            c.env.principal().to_string(),
            c.env.action().to_string(),
            c.env.resource().to_string(),
            c.id.to_string(),
        )
    });
    Ok(out)
}

/// The source cubes grouped by environment, as JSON.
pub fn cubes_to_json(cubes: &[SourceCube]) -> serde_json::Value {
    serde_json::Value::Array(cubes.iter().map(SourceCube::to_json).collect())
}

impl SourceCube {
    /// The cube as a policy over its environment:
    /// `permit(principal is pt, action == at, resource is rt) when { condition }`,
    /// annotated `@woodpecker("cube <id>")`.
    pub fn to_policy(&self) -> Result<cedar_policy::Policy, Error> {
        let when = match self.condition.expr_kind() {
            cedar_policy_core::ast::ExprKind::Lit(cedar_policy_core::ast::Literal::Bool(true)) => {
                None
            }
            _ => Some(self.condition.clone()),
        };
        crate::escalate::permit(
            self.id.as_ref(),
            crate::escalate::annotation(&format!("cube {}", self.id))?,
            self.env.principal(),
            self.env.action(),
            self.env.resource(),
            when,
        )
        .map(cedar_policy::Policy::from)
    }
}

/// The source cubes as Cedar text: per environment a comment, then every cube
/// of it as a policy, each preceded by a comment with its id and outcomes.
pub fn cubes_to_cedar(cubes: &[SourceCube]) -> Result<String, Error> {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut current: Option<&RequestEnv> = None;
    for cube in cubes {
        if current != Some(&cube.env) {
            let _ = writeln!(out, "// {}", env_name(&cube.env));
            current = Some(&cube.env);
        }
        let _ = writeln!(
            out,
            "// {} [{}]\n{}",
            cube.id,
            outcome_names(&cube.outcomes).join(", "),
            cube.to_policy()?.to_cedar().unwrap_or_default()
        );
    }
    Ok(out)
}
