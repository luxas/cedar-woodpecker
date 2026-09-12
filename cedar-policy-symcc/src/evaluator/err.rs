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

//! Errors of the symbolic evaluator.

use cedar_policy::RequestEnv;
use cedar_policy_core::validator::{types::Type, ValidationError};
use miette::Diagnostic;
use thiserror::Error;

/// Infrastructure errors of the symbolic evaluator.
///
/// None of these variants means "the evaluated expression errors": that is a
/// normal result, reported through the possible outcomes of the returned
/// expression (see [`super::EvaluationMetadata`]).
#[derive(Debug, Diagnostic, Error)]
pub enum EvaluationError {
    /// The request environment is not declared in the schema.
    #[error("request env not found in the schema: {0:?}")]
    RequestEnvNotFound(RequestEnv),
    /// The target expression is not well typed in the request environment.
    #[error("expression is not well typed with respect to the schema in this request env")]
    NotWellTyped {
        /// Errors from the typechecker.
        #[related]
        errs: Vec<ValidationError>,
    },
    /// A typechecked policy set was built against a different schema than the
    /// evaluator's; its cached typed conditions do not apply.
    #[error(
        "the typechecked policy set was built against a different schema than the evaluator's"
    )]
    SchemaMismatch,
    /// The target expression is well typed but not boolean.
    #[error("expression has type {ty}, expected a boolean")]
    NotBoolean {
        /// Type the typechecker assigned to the expression.
        ty: Type,
    },
    /// An assumption registered with `assume_*` is not well typed in the
    /// request environment being evaluated.
    #[error("assumption is not well typed with respect to the schema in request env {env:?}")]
    AssumptionNotWellTyped {
        /// The request environment.
        env: RequestEnv,
        /// Errors from the typechecker.
        #[related]
        errs: Vec<ValidationError>,
    },
    /// The assumptions are contradictory in this request environment: no
    /// well-formed input satisfies them, so nothing can be said about the
    /// expression.
    #[error("assumptions are unsatisfiable in this request env")]
    UnsatisfiableAssumptions,
    /// Entity data passed to `assume_entities` / `assume_partial_entities`
    /// does not fit the schema.
    #[error("invalid entities: {0}")]
    InvalidEntities(String),
    /// A request passed to `assume_request` / `assume_partial_request` does
    /// not fit the schema.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The expression nests too deeply (same limit as the concrete evaluator).
    #[error("recursion limit reached while evaluating the expression")]
    RecursionLimit,
    /// An internal invariant was violated.
    #[error("internal error in the symbolic evaluator: {0}")]
    Internal(&'static str),
    /// Errors from symbolic compilation or the solver.
    #[error(transparent)]
    SymCC(#[from] crate::Error),
}

/// A result type for the symbolic evaluator.
pub type Result<T> = std::result::Result<T, EvaluationError>;
