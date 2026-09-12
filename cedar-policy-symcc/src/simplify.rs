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

//! The policy simplifier: where in a policy an expression is statically
//! true or false (or always an error), pinpointed with the symbolic
//! evaluator.
//!
//! For every static policy that validates, and every request environment of
//! the schema, the [`Evaluator`] computes the outcomes each visited node of
//! the policy condition can take under the *trail* — the assumptions implied
//! by the path to it (`a` was true when `b` in `a && b` runs). Nodes are keyed
//! by source span, so environments are aligned by position rather than by
//! tree shape. Walking the spans in evaluation order (post-order by source
//! position):
//!
//! * a span is a [`Finding`] when, in every environment still under
//!   consideration, its outcome set lacks `True` (never true), or in every one
//!   lacks `False` (never false) — and no span inside it is a finding already:
//!   the innermost cause is reported, its ancestors are implied;
//! * after a span is processed, every environment in which it can never be
//!   true is dropped for the spans evaluated later: the policy is false there
//!   already, so those nodes' outcomes in it do not matter;
//! * a `has`/`hasTag` guard the evaluator *keeps* (plan 5: of an optional
//!   attribute or a tag, with its access in the right operand of the `&&` or
//!   the `then` branch of the `if` it guards) is not reported as never false
//!   (plan 7): however true the assumptions make it, strict validation
//!   needs it, so the warning would only invite a change the validator
//!   rejects. It is still reported when never true. An `||` needs no such
//!   rule: the validator gives its right operand no capability from its left.
//!
//! Each static policy is typechecked once, in every environment, as a
//! one-policy [`cedar_policy::TypecheckedPolicySet`] (plan 3); the evaluator takes its
//! cached typed condition. Policies that do not validate, template-linked
//! policies and unlinked templates are skipped.

use std::collections::BTreeMap;
use std::str::FromStr;

use cedar_policy::{PolicySet, RequestEnv, Schema, Typecheckable};
use cedar_policy_core::ast::{Expr, ExprKind};
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use serde::Serialize;

use nonempty_collections::NESet;

use crate::evaluator::{EvaluationError, Evaluator, NodeOutcome};
use crate::semantics::Assumption;
use crate::solver::Solver;

/// What a span can never be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    /// The expression can never be `true`: it is `false`, or an error.
    NeverTrue,
    /// The expression can never be `false`: it is `true`, or an error.
    NeverFalse,
    /// The expression always errors.
    AlwaysError,
}

/// A statically determined expression of a policy document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Byte offset of the expression in the document.
    pub offset: usize,
    /// Byte length of the expression in the document.
    pub length: usize,
    /// What the expression can never be.
    pub kind: FindingKind,
    /// A message for the user.
    pub message: String,
}

/// Errors of the simplifier (a policy that does not validate is not an
/// error: it is skipped).
#[derive(Debug, thiserror::Error)]
pub enum SimplifyError {
    /// The policy document does not parse.
    #[error("the policies do not parse: {0}")]
    Parse(String),
    /// A `@semantics` assumption is not well typed (or not boolean) in some
    /// request environment: it needs an `if <var> is <type> then … else true`
    /// guard.
    #[error("the @semantics assumption on {origin} (`{expression}`) is not well typed in the request environment (principal: {principal}, action: {action}, resource: {resource}): {message}")]
    AssumptionNotWellTyped {
        /// The declaration the annotation is on.
        origin: String,
        /// The assumption as written.
        expression: String,
        /// The environment's principal type.
        principal: String,
        /// The environment's action.
        action: String,
        /// The environment's resource type.
        resource: String,
        /// The typechecker's message.
        message: String,
    },
    /// The `@semantics` assumptions contradict each other (or the schema) in
    /// some request environment, where everything would then be vacuously
    /// determined.
    #[error("the @semantics assumptions are unsatisfiable in the request environment (principal: {principal}, action: {action}, resource: {resource})")]
    UnsatisfiableAssumptions {
        /// The environment's principal type.
        principal: String,
        /// The environment's action.
        action: String,
        /// The environment's resource type.
        resource: String,
    },
    /// The symbolic evaluator failed.
    #[error("symbolic evaluation failed: {0}")]
    Evaluation(#[from] EvaluationError),
}

/// The three names of a request environment, for messages.
fn env_names(env: &RequestEnv) -> (String, String, String) {
    (
        env.principal().to_string(),
        env.action().to_string(),
        env.resource().to_string(),
    )
}

/// A source span, `(offset, length)`.
type Span = (usize, usize);

/// A `&&` in *conjunctive position* — reachable from the root through `&&`
/// only — with the spans of its operands (the left one is absent for a scope
/// constraint, which has no source location).
#[derive(Debug, Clone, Copy)]
struct Conjunction {
    span: Span,
    left: Option<Span>,
    right: Span,
}

/// The conjunctions of `e`. When one can never be true because its *left*
/// operand cannot, the whole condition can never be true (the policy is
/// false) and its right operand does not run.
fn conjunctions(e: &Expr, out: &mut Vec<Conjunction>) {
    if let ExprKind::And { left, right } = e.expr_kind() {
        let span_of = |x: &Expr| x.source_loc().map(|l| (l.span.offset(), l.span.len()));
        if let (Some(span), Some(right_span)) = (span_of(e), span_of(right)) {
            out.push(Conjunction {
                span,
                left: span_of(left),
                right: right_span,
            });
        }
        conjunctions(left, out);
        conjunctions(right, out);
    }
}

/// What is recorded about a visited node: its outcome set, whether it is a
/// literal the user wrote, whether it is a kept guard.
#[derive(Debug, Clone, Copy)]
struct Observed {
    set: OutcomeSet,
    literal: bool,
    kept: bool,
}

/// The outcome sets of the visited nodes of an evaluated condition, by span.
/// A literal the user wrote is its own explanation and is not a node to
/// report; it is still recorded (a folded root is one), flagged.
fn outcomes_by_span(observed: &[NodeOutcome]) -> BTreeMap<Span, Observed> {
    observed
        .iter()
        .filter_map(|node| {
            let loc = node.loc.as_ref()?;
            Some((
                (loc.span.offset(), loc.span.len()),
                Observed {
                    set: OutcomeSet::of(&node.outcomes),
                    literal: node.literal,
                    kept: node.kept,
                },
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutcomeSet {
    can_true: bool,
    can_false: bool,
    can_error: bool,
}

impl OutcomeSet {
    fn of(outcomes: &NESet<EvaluationOutcome>) -> Self {
        Self {
            can_true: outcomes.contains(&EvaluationOutcome::True),
            can_false: outcomes.contains(&EvaluationOutcome::False),
            can_error: outcomes.contains(&EvaluationOutcome::Error),
        }
    }
}

/// Runs the simplifier over every policy of `policies` (a policy document)
/// against `schema`, under the schema's `@semantics` `assumptions` (see
/// [`crate::semantics`]), each assumed `true` in every request environment
/// — so each must typecheck in every one, which is checked first.
pub async fn simplify<S: Solver + Send>(
    mut evaluator: Evaluator<S>,
    schema: &Schema,
    policies: &str,
    assumptions: &[Assumption],
) -> Result<(Vec<Finding>, Evaluator<S>), SimplifyError> {
    let pset = PolicySet::from_str(policies).map_err(|e| SimplifyError::Parse(e.to_string()))?;
    let envs: Vec<_> = schema.request_envs().collect();
    for env in &envs {
        for assumption in assumptions {
            if let Err(e) = evaluator.typecheck_assumption(&assumption.expression, env) {
                let (principal, action, resource) = env_names(env);
                return Err(SimplifyError::AssumptionNotWellTyped {
                    origin: assumption.origin.clone(),
                    expression: assumption.source.clone(),
                    principal,
                    action,
                    resource,
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
    let extras: Vec<Expr> = assumptions.iter().map(|a| a.expression.clone()).collect();
    // an unsatisfiable set is an error even when no policy gets evaluated
    // (an empty or invalid document): evaluating `true` under the
    // assumptions asks the solver exactly that, once per environment
    if !extras.is_empty() {
        for env in &envs {
            match evaluator
                .evaluate_observed(&Expr::val(true), env, extras.iter().cloned())
                .await
            {
                Ok(_) => {}
                Err(EvaluationError::UnsatisfiableAssumptions) => {
                    let (principal, action, resource) = env_names(env);
                    return Err(SimplifyError::UnsatisfiableAssumptions {
                        principal,
                        action,
                        resource,
                    });
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    let mut findings = Vec::new();
    for policy in pset.policies() {
        let ast: &cedar_policy_core::ast::Policy = policy.as_ref();
        if !ast.is_static() {
            continue;
        }
        // one policy at a time: the validator reports on the set as a whole,
        // and a policy that does not validate is skipped, not an error
        let single = PolicySet::from_policies([policy.clone()])
            .map_err(|e| SimplifyError::Parse(e.to_string()))?;
        let Ok(typechecked) = single.typecheck(schema) else {
            continue;
        };
        let Some(typechecked) = typechecked.get(&policy.id().clone()) else {
            continue;
        };
        // a policy without `when`/`unless` has nothing to simplify: its
        // condition is only its scope, and the typechecker would fold
        // `permit(principal, action, resource);` to `true` — reported as a
        // whole policy that is always true
        let has_condition = ast.non_scope_constraints().is_some_and(|e| {
            !matches!(
                e.expr_kind(),
                ExprKind::Lit(cedar_policy_core::ast::Literal::Bool(true))
            )
        });
        if !has_condition {
            continue;
        }
        let mut conjunctive: Vec<Conjunction> = Vec::new();
        conjunctions(typechecked.untyped(), &mut conjunctive);
        // per environment, the outcome sets by span; the evaluator takes the
        // typed condition the set holds for the environment
        let mut per_env: Vec<BTreeMap<Span, Observed>> = Vec::new();
        for env in &envs {
            // unsatisfiable assumptions were rejected above
            let (_, mut observed) = evaluator
                .evaluate_observed(typechecked, env, extras.iter().cloned())
                .await?;
            // the root is the policy: when the typechecker folded the
            // whole condition to a literal, that literal *is* the policy
            // (its location is the policy's, from the condition's `&&`
            // chain, or none at all) — a finding, not a literal the user
            // wrote
            if let Some(root) = observed.last_mut() {
                if root.literal || root.loc.is_none() {
                    root.loc = ast.loc().cloned();
                    root.literal = false;
                }
            }
            per_env.push(outcomes_by_span(&observed));
        }
        findings.extend(analyze(&per_env, &conjunctive));
    }
    Ok((findings, evaluator))
}

/// The findings of one policy from its per-environment outcome sets.
///
/// A span is judged over the environments in which it is evaluated. An
/// environment that does not evaluate it is *ignored* for it when the span
/// lies in the right operand of a conjunction whose left operand can never
/// be true there: the policy is false in that environment — a scope it
/// cannot satisfy, or a conjunct that is never true — and what follows does
/// not run (the typechecker folds it away, or the evaluator never reaches
/// it). That is the
/// plan's "drop an environment once the policy is false there"; the conjunct
/// that made it false is judged like any other node, in every environment
/// that evaluates it. An environment that does not evaluate the span for any
/// other reason — the typechecker folded it away as true, a disjunction
/// short-circuited — blocks the finding: a warning speaks for every
/// environment. Literals the user wrote are not reported, nor is a guard
/// kept in any environment reported as never false.
fn analyze(per_env: &[BTreeMap<Span, Observed>], conjunctive: &[Conjunction]) -> Vec<Finding> {
    // every span seen, in evaluation order: post-order by source position —
    // a node completes after its children, so by end offset, shorter spans
    // (descendants) before longer ones ending at the same place
    let mut spans: Vec<Span> = per_env.iter().flat_map(|m| m.keys().copied()).collect();
    spans.sort_by_key(|&(offset, length)| (offset + length, length));
    spans.dedup();
    let contains =
        |outer: Span, inner: Span| outer.0 <= inner.0 && inner.0 + inner.1 <= outer.0 + outer.1;
    // per environment, the right operands of the conjunctions whose *left*
    // operand can never be true there: what does not run. A left operand
    // the typechecker folded away (`false && x` becomes a literal at the
    // `&&`), or that has no location (a scope constraint), shows as the
    // `&&` being never true with nothing of its right operand recorded.
    let never_true =
        |m: &BTreeMap<Span, Observed>, span: &Span| m.get(span).is_some_and(|o| !o.set.can_true);
    let dead: Vec<Vec<Span>> = per_env
        .iter()
        .map(|m| {
            conjunctive
                .iter()
                .filter(|c| {
                    never_true(m, &c.span)
                        && match c.left {
                            Some(left) if m.contains_key(&left) => never_true(m, &left),
                            _ => !m.contains_key(&c.right),
                        }
                })
                .map(|c| c.right)
                .collect()
        })
        .collect();
    let mut findings: Vec<Finding> = Vec::new();
    let mut reported: Vec<Span> = Vec::new();
    for span in spans {
        let mut sets: Vec<OutcomeSet> = Vec::new();
        let mut blocked = false;
        let mut literal = false;
        let mut kept = false;
        for (m, dead_here) in per_env.iter().zip(&dead) {
            match m.get(&span) {
                Some(o) => {
                    sets.push(o.set);
                    literal |= o.literal;
                    kept |= o.kept;
                }
                None if dead_here.iter().any(|&d| contains(d, span)) => {}
                None => blocked = true,
            }
        }
        if blocked || sets.is_empty() || literal {
            continue;
        }
        let never_true = sets.iter().all(|s| !s.can_true);
        // a kept guard is needed however true it is (plan 7)
        let never_false = sets.iter().all(|s| !s.can_false) && !kept;
        let inner_reported = reported.iter().any(|&r| contains(span, r));
        if (never_true || never_false) && !inner_reported {
            let kind = if never_true && never_false {
                FindingKind::AlwaysError
            } else if never_true {
                FindingKind::NeverTrue
            } else {
                FindingKind::NeverFalse
            };
            let can_error = sets.iter().any(|s| s.can_error);
            let what = match (kind, can_error) {
                (FindingKind::AlwaysError, _) => "always errors",
                (FindingKind::NeverTrue, true) => "is never true (it is false, or errors)",
                (FindingKind::NeverTrue, false) => "is always false",
                (FindingKind::NeverFalse, true) => "is never false (it is true, or errors)",
                (FindingKind::NeverFalse, false) => "is always true",
            };
            let message = format!(
                "this expression {what} in every request environment, given what is evaluated before it"
            );
            findings.push(Finding {
                offset: span.0,
                length: span.1,
                kind,
                message,
            });
            reported.push(span);
        }
    }
    findings
}
