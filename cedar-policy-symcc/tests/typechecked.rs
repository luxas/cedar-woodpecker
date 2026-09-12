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

//! The symbolic evaluator over pre-typechecked expressions and policy sets
//! (plans 3 and 5): the `Typecheckable` seam, `TypecheckedPolicySet`, `typed_of`
//! and `symbolic_fold`.

#![expect(clippy::unwrap_used, clippy::panic, reason = "test code")]

mod utils;

use std::str::FromStr;
use std::sync::Arc;

use cedar_policy::{EvaluationOutcome, Expression, PolicyId, PolicySet, RequestEnv, Schema};
use cedar_policy_core::ast::Expr;
use cedar_policy_core::typechecked::typecheck_expr;
use cedar_policy_core::validator::ValidationMode;
use cedar_policy_symcc::evaluator::{erase_metadata, typed_of, EvaluationError, Evaluator};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::typechecked::{Pretyped, SymbolicFold};
use cedar_policy_symcc::CedarSymCompiler;
use nonempty_collections::nes;

use utils::{req_env_from_strs, schema_from_cedarstr};

const SCHEMA: &str = r#"
entity Group in [Group];
entity User in [Group] { name: String, active: Bool, age: Long, nick?: String };
entity Document { protected: Bool, owner: User, level: Long };
action view appliesTo { principal: User, resource: Document };
action edit appliesTo { principal: User, resource: Document };
action admin appliesTo { principal: Group, resource: Document };
"#;

const POLICIES: &str = r#"
permit(principal, action == Action::"view", resource) when { !resource.protected };
permit(principal, action, resource) when { resource.owner == principal };
forbid(principal is User, action, resource) when { principal.age < 18 && resource.level > 1 };
permit(principal is Group, action == Action::"admin", resource);
"#;

fn schema() -> Schema {
    schema_from_cedarstr(SCHEMA)
}

fn view() -> RequestEnv {
    req_env_from_strs("User", r#"Action::"view""#, "Document")
}

fn expr(text: &str) -> Expr {
    Expression::from_str(text).unwrap().as_ref().clone()
}

fn no_extra() -> Vec<Expr> {
    Vec::new()
}

fn evaluator(schema: &Schema) -> Evaluator<LocalSolver> {
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap()).unwrap();
    Evaluator::new(compiler, schema).unwrap()
}

fn policies() -> PolicySet {
    PolicySet::from_str(POLICIES).unwrap()
}

/// A typed expression without its types (structure preserved).
fn erase_typed(e: &Expr<Option<cedar_policy_core::validator::types::Type>>) -> Expr {
    expr(&format!("{e}"))
}

/// The typed form of `text` in `env`, straight from the core typechecker.
fn typed(
    schema: &Schema,
    env: &RequestEnv,
    text: &str,
) -> Arc<Expr<Option<cedar_policy_core::validator::types::Type>>> {
    let validator_schema = schema.as_ref();
    let validator_env = validator_schema
        .unlinked_request_envs(ValidationMode::Strict)
        .find(|e| {
            e.principal_entity_type() == Some(env.principal().as_ref())
                && e.action_entity_uid() == Some(env.action().as_ref())
                && e.resource_entity_type() == Some(env.resource().as_ref())
        })
        .unwrap();
    Arc::new(typecheck_expr(&expr(text), &validator_env, validator_schema).unwrap())
}

/// (a) A pre-typed expression is evaluated without running the typechecker:
/// the result is the plain path's, and an untyped form the typechecker
/// would reject does not matter.
#[tokio::test]
async fn pretyped_skips_the_typechecker() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let text = "principal.age > 18 && resource.protected";
    let plain = ev.evaluate(&expr(text), &view(), no_extra()).await.unwrap();
    let queries = ev.solver_queries();

    let pre = Pretyped::new(expr("principal.nonexistent"), typed(&schema, &view(), text));
    let result = ev.evaluate(&pre, &view(), no_extra()).await.unwrap();
    assert_eq!(
        erase_metadata(&result).unwrap(),
        erase_metadata(&plain).unwrap()
    );
    assert_eq!(result.data().outcomes(), plain.data().outcomes());
    assert_eq!(ev.solver_queries() - queries, queries, "the same queries");

    // The plain path does typecheck.
    assert!(matches!(
        ev.evaluate(&expr("principal.nonexistent"), &view(), no_extra())
            .await,
        Err(EvaluationError::NotWellTyped { .. })
    ));
}

/// (b) Every policy of a typechecked set evaluates, in every environment, to
/// what its condition evaluates to on the plain path, and equivalently.
#[tokio::test]
async fn typechecked_policies_evaluate_like_their_conditions() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let tps = policies().typecheck(&schema).unwrap();
    assert_eq!(tps.policies().count(), 4);
    for policy in tps.policies() {
        let condition = tps.policy(&policy.id()).unwrap().as_ref().condition();
        for env in policy.request_envs() {
            let cached = ev.evaluate(policy, &env, no_extra()).await.unwrap();
            let plain = ev.evaluate(&condition, &env, no_extra()).await.unwrap();
            assert_eq!(
                erase_metadata(&cached).unwrap(),
                erase_metadata(&plain).unwrap(),
                "policy {} in {env:?}",
                policy.id()
            );
            assert_eq!(cached.data().outcomes(), plain.data().outcomes());
            assert!(ev
                .check_equivalent(policy, &cached, &env, no_extra())
                .await
                .unwrap());
            // What the set knew statically is an over-approximation of what
            // the evaluator found.
            for o in cached.data().outcomes().iter() {
                assert!(policy.outcomes(&env).unwrap().contains(o));
            }
        }
    }
}

/// (c) On another schema the cached conditions do not apply; the fallback
/// typechecks the condition afresh and agrees with the plain path.
#[tokio::test]
async fn other_schema_takes_the_fallback() {
    let small = schema();
    let bigger = schema_from_cedarstr(&format!("{SCHEMA}\nentity Other;"));
    let tps = policies().typecheck(&small).unwrap();
    let policy = tps.get(&PolicyId::new("policy0")).unwrap();
    assert!(!policy.same_schema(&bigger));
    let mut ev = evaluator(&bigger);
    let condition = tps.policy(&policy.id()).unwrap().as_ref().condition();
    let cached = ev.evaluate(policy, &view(), no_extra()).await.unwrap();
    let plain = ev.evaluate(&condition, &view(), no_extra()).await.unwrap();
    assert_eq!(
        erase_metadata(&cached).unwrap(),
        erase_metadata(&plain).unwrap()
    );
}

/// (d) `symbolic_fold` stores the evaluator's residual, typed, and its
/// outcomes; folding again changes nothing.
#[tokio::test]
async fn symbolic_fold_stores_typed_residuals_and_outcomes() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("!resource.protected"));
    ev.assume_expr(expr(
        "if principal is User then principal.age >= 18 else true",
    ));
    let tps = policies()
        .typecheck(&schema)
        .unwrap()
        .symbolic_fold(&mut ev)
        .await
        .unwrap();

    // `permit(…, action == Action::"view", …) when { !resource.protected }`
    // is `true` in a `view` environment under the assumption.
    let p0 = tps.get(&PolicyId::new("policy0")).unwrap();
    let folded = p0.typed_condition(&view()).unwrap().as_ref().unwrap();
    assert_eq!(erase_typed(folded), expr("true"));
    assert!(
        folded.data().is_some(),
        "the folded literal keeps its node's type"
    );
    assert_eq!(
        p0.outcomes(&view()).unwrap(),
        &nes![EvaluationOutcome::True]
    );
    // … and `false` in an `edit` environment: the action constraint.
    let edit = req_env_from_strs("User", r#"Action::"edit""#, "Document");
    assert_eq!(p0.outcomes(&edit).unwrap(), &nes![EvaluationOutcome::False]);

    // The forbid cannot fire for an adult (it may still error: the guarded
    // assumption derives no existence fact for `principal`).
    let p2 = tps.get(&PolicyId::new("policy2")).unwrap();
    assert!(!p2
        .outcomes(&view())
        .unwrap()
        .contains(&EvaluationOutcome::True));

    // The owner check stays open; the fold keeps its type on every node.
    let p1 = tps.get(&PolicyId::new("policy1")).unwrap();
    let folded = p1.typed_condition(&view()).unwrap().as_ref().unwrap();
    assert_eq!(
        format!("{folded}"),
        format!("{}", expr("resource.owner == principal"))
    );
    assert!(folded.data().is_some());
    assert_eq!(
        p1.outcomes(&view()).unwrap(),
        &nes![EvaluationOutcome::True, EvaluationOutcome::False]
    );

    // Idempotent: the folded set evaluates to itself.
    let before: Vec<_> = tps
        .policies()
        .flat_map(|p| {
            p.request_envs()
                .map(|e| format!("{:?}", p.typed_condition(&e)))
                .collect::<Vec<_>>()
        })
        .collect();
    let again = tps.symbolic_fold(&mut ev).await.unwrap();
    let after: Vec<_> = again
        .policies()
        .flat_map(|p| {
            p.request_envs()
                .map(|e| format!("{:?}", p.typed_condition(&e)))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(before, after);
}

/// A set built against another schema is refused by the fold.
#[tokio::test]
async fn symbolic_fold_needs_the_evaluators_schema() {
    let bigger = schema_from_cedarstr(&format!("{SCHEMA}\nentity Other;"));
    let mut ev = evaluator(&bigger);
    let tps = policies().typecheck(&schema()).unwrap();
    assert!(matches!(
        tps.symbolic_fold(&mut ev).await,
        Err(EvaluationError::SchemaMismatch)
    ));
}

/// (e) `typed_of` types every node of a result and erases to the residual.
#[tokio::test]
async fn typed_of_types_every_node() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let result = ev
        .evaluate(
            &expr("principal.age > 18 && (resource.protected || resource.level == 3)"),
            &view(),
            no_extra(),
        )
        .await
        .unwrap();
    let typed = typed_of(&result).unwrap();
    assert_eq!(
        format!("{typed}"),
        format!("{}", erase_metadata(&result).unwrap())
    );
    // Re-evaluating the typed residual as a pre-typed expression gives the
    // same result: every node is typed, the root is boolean.
    let pre = Pretyped::new(erase_metadata(&result).unwrap(), Arc::new(typed));
    let again = ev.evaluate(&pre, &view(), no_extra()).await.unwrap();
    assert_eq!(
        erase_metadata(&again).unwrap(),
        erase_metadata(&result).unwrap()
    );
    assert_eq!(again.data().outcomes(), result.data().outcomes());
}

/// `constant_fold` then `symbolic_fold`: the TPE-folded conditions are what
/// the evaluator starts from, and the answers agree with the unfolded set.
#[tokio::test]
async fn constant_fold_then_symbolic_fold_agree() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr(
        "if principal is User then principal.age >= 18 else true",
    ));
    let plain = policies()
        .typecheck(&schema)
        .unwrap()
        .symbolic_fold(&mut ev)
        .await
        .unwrap();
    let folded = policies()
        .typecheck(&schema)
        .unwrap()
        .constant_fold()
        .symbolic_fold(&mut ev)
        .await
        .unwrap();
    for (a, b) in plain.policies().zip(folded.policies()) {
        assert_eq!(a.id(), b.id());
        for env in a.request_envs() {
            assert_eq!(a.outcomes(&env), b.outcomes(&env), "{} in {env:?}", a.id());
        }
    }
}
