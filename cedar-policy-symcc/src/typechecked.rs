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

//! The symbolic evaluator over a [`TypecheckedPolicySet`]: folding every
//! policy's condition in every request environment under the evaluator's
//! assumptions, keeping the result typed and its outcomes.
//!
//! [`Evaluator::evaluate`] takes any [`Typecheckable`]; a
//! [`cedar_policy::TypecheckedPolicy`] hands it the cached typed condition,
//! so no typechecking happens on the way (plan 3).

use std::future::Future;
use std::sync::Arc;

use cedar_policy::{Typecheckable, TypecheckedPolicySet};

use crate::evaluator::{typed_of, EvaluationError, Evaluator};
use crate::solver::Solver;

/// Folding a typechecked policy set with the symbolic evaluator.
pub trait SymbolicFold: Sized {
    /// Evaluates every policy's condition in every request environment under
    /// `evaluator`'s assumptions and replaces it by the result: the residual
    /// with every node typed, and the outcomes the evaluator established. An
    /// environment whose condition is already a static error is left alone.
    ///
    /// The set must have been built against the evaluator's schema
    /// ([`EvaluationError::SchemaMismatch`] otherwise).
    fn symbolic_fold<S: Solver + Send>(
        self,
        evaluator: &mut Evaluator<S>,
    ) -> impl Future<Output = Result<Self, EvaluationError>>;
}

impl SymbolicFold for TypecheckedPolicySet {
    async fn symbolic_fold<S: Solver + Send>(
        mut self,
        evaluator: &mut Evaluator<S>,
    ) -> Result<Self, EvaluationError> {
        let schema = evaluator.schema().clone();
        for policy in self.policies_mut() {
            if !policy.same_schema(&schema) {
                return Err(EvaluationError::SchemaMismatch);
            }
            let envs: Vec<_> = policy.request_envs().collect();
            for env in envs {
                if !matches!(policy.typed_condition(&env), Some(Ok(_))) {
                    continue;
                }
                let result = evaluator
                    .evaluate(&*policy, &env, std::iter::empty())
                    .await?;
                let outcomes = result.data().outcomes().clone();
                let typed = typed_of(&result)?;
                policy
                    .fold_env(&env, Arc::new(typed), outcomes)
                    .map_err(|e| match e {
                        cedar_policy::TypecheckError::NotWellTyped { errs } => {
                            EvaluationError::NotWellTyped { errs }
                        }
                        _ => EvaluationError::Internal("folded condition is not typed"),
                    })?;
            }
        }
        Ok(self)
    }
}

/// A [`Typecheckable`] whose typed form is fixed in advance: the evaluator
/// runs no typechecker on it. For callers that hold a typed expression from
/// elsewhere (a test, a pass that already typechecked).
#[derive(Debug, Clone)]
pub struct Pretyped {
    untyped: cedar_policy_core::ast::Expr,
    typed: Arc<cedar_policy_core::ast::Expr<Option<cedar_policy_core::validator::types::Type>>>,
}

impl Pretyped {
    /// `typed` must be the typed form of `untyped` in every environment it is
    /// evaluated in.
    pub fn new(
        untyped: cedar_policy_core::ast::Expr,
        typed: Arc<cedar_policy_core::ast::Expr<Option<cedar_policy_core::validator::types::Type>>>,
    ) -> Self {
        Self { untyped, typed }
    }
}

impl Typecheckable for Pretyped {
    fn untyped(&self) -> &cedar_policy_core::ast::Expr {
        &self.untyped
    }

    fn typechecked(
        &self,
        _env: &cedar_policy_core::validator::types::RequestEnv<'_>,
        _schema: &cedar_policy_core::validator::ValidatorSchema,
    ) -> Result<
        Arc<cedar_policy_core::ast::Expr<Option<cedar_policy_core::validator::types::Type>>>,
        cedar_policy::TypecheckError,
    > {
        Ok(Arc::clone(&self.typed))
    }
}
