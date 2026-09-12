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

//! Tests for the symbolic evaluator.
//!
//! The expected residuals and outcome sets below are derived by hand from
//! Cedar's three-valued, short-circuiting evaluation semantics. They are not
//! snapshots: if the evaluator changes behaviour, fix the evaluator or argue
//! the table is wrong. Every expected residual is also checked against the
//! input with the solver (`check_equivalent`), so a wrong expectation that the
//! evaluator happens to reproduce is caught as well.

#![expect(clippy::unwrap_used, clippy::panic, reason = "test code")]

mod utils;

use std::str::FromStr;

use cedar_policy::{Decision, Expression, RequestEnv, Schema};
use cedar_policy_core::ast::{Expr, ExprKind, PolicySet};
use cedar_policy_core::parser::parse_policy;
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_symcc::evaluator::{
    erase_metadata, literal_of, EvaluationError, EvaluationMetadata, Evaluator,
};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;
use nonempty_collections::{nes, NESet};

use utils::{req_env_from_strs, schema_from_cedarstr};

const SCHEMA: &str = r#"
entity Group in [Group];
entity User in [Group] { name: String, active: Bool, age: Long, nick?: String, flag?: Bool } tags Long;
entity Document { protected: Bool, owner: User, level: Long };
action view appliesTo { principal: User, resource: Document };
action edit appliesTo { principal: User, resource: Document };
action admin appliesTo { principal: Group, resource: Document };
"#;

fn schema() -> Schema {
    schema_from_cedarstr(SCHEMA)
}

fn view() -> RequestEnv {
    req_env_from_strs("User", r#"Action::"view""#, "Document")
}

fn edit() -> RequestEnv {
    req_env_from_strs("User", r#"Action::"edit""#, "Document")
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

fn outcomes(set: &[EvaluationOutcome]) -> NESet<EvaluationOutcome> {
    let mut iter = set.iter().cloned();
    let mut res = nes![iter.next().unwrap()];
    for o in iter {
        res.insert(o);
    }
    res
}

use EvaluationOutcome::{Error as E, False as F, True as T};

/// Evaluates `text` in `env`, checks the result is equivalent to the input
/// under the assumptions, and returns it.
async fn eval(
    evaluator: &mut Evaluator<LocalSolver>,
    text: &str,
    env: &RequestEnv,
) -> Expr<EvaluationMetadata> {
    let e = expr(text);
    let result = evaluator
        .evaluate(&e, env, no_extra())
        .await
        .unwrap_or_else(|err| panic!("evaluating `{text}`: {err}"));
    assert!(
        evaluator
            .check_equivalent(&e, &result, env, no_extra())
            .await
            .unwrap(),
        "result `{result}` is not equivalent to `{text}` under the assumptions"
    );
    result
}

#[track_caller]
fn assert_residual(
    result: &Expr<EvaluationMetadata>,
    expected: &str,
    expected_outcomes: &[EvaluationOutcome],
) {
    assert_eq!(
        erase_metadata(result).unwrap(),
        expr(expected),
        "expected residual `{expected}`, got `{result}`"
    );
    assert_eq!(
        result.data().outcomes(),
        &outcomes(expected_outcomes),
        "outcomes of `{result}`"
    );
}

#[track_caller]
fn assert_literal(result: &Expr<EvaluationMetadata>, expected: bool) {
    assert_eq!(
        literal_of(result),
        Some(expected),
        "expected literal {expected}, got `{result}`"
    );
    assert_eq!(
        result.data().outcomes(),
        &outcomes(&[if expected { T } else { F }])
    );
}

#[tokio::test]
async fn readme_example() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr(r#"principal == User::"lucas""#));
    ev.assume_expr(expr(r#"Document::"foo".protected == false"#));

    // `resource` is still symbolic: only the principal half folds.
    let r = eval(
        &mut ev,
        r#"principal == User::"lucas" && !resource.protected"#,
        &view(),
    )
    .await;
    // (`resource` may not even exist: an error is possible)
    assert_residual(&r, "!resource.protected", &[T, F, E]);

    // Pinning `resource` folds everything.
    ev.assume_expr(expr(r#"resource == Document::"foo""#));
    let r = eval(
        &mut ev,
        r#"principal == User::"lucas" && !resource.protected"#,
        &view(),
    )
    .await;
    assert_literal(&r, true);
}

#[tokio::test]
async fn atoms_without_assumptions() {
    let schema = schema();
    let mut ev = evaluator(&schema);

    // required attribute: error-free — once the principal is known to exist
    // (nothing says so here, so the access may error on a missing entity)
    let r = eval(&mut ev, "principal.active", &view()).await;
    assert_residual(&r, "principal.active", &[T, F, E]);

    // arithmetic may overflow: may error. (An unguarded optional attribute
    // access would be the other candidate, but strict validation rejects it.)
    let r = eval(&mut ev, "principal.age + 1 > 18", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18", &[T, F, E]);

    // the trail carries the `has` into the right operand, which then cannot
    // miss the attribute — nor the entity: only a present entity has
    // attributes, so a true `has` proves the principal exists
    let r = eval(&mut ev, "principal has flag && principal.flag", &view()).await;
    assert_residual(&r, "principal has flag && principal.flag", &[T, F]);
    let ExprKind::And { left, right } = r.expr_kind() else {
        panic!("expected an `&&`, got `{r}`");
    };
    assert_eq!(left.data().outcomes(), &outcomes(&[T, F]));
    assert_eq!(right.data().outcomes(), &outcomes(&[T, F]));
}

#[tokio::test]
async fn three_valued_and_or() {
    let schema = schema();
    let mut ev = evaluator(&schema);

    // `<can-error> && false` cannot fold: it is false or an error
    let r = eval(&mut ev, "principal.age + 1 > 18 && false", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18 && false", &[F, E]);

    // `<error-free> && false` is false — once the principal is known to
    // exist; without that, `principal.active` may error
    let r = eval(&mut ev, "principal.active && false", &view()).await;
    assert_residual(&r, "principal.active && false", &[F, E]);
    ev.assume_expr(expr("principal.active || true"));
    let r = eval(&mut ev, "principal.active && false", &view()).await;
    assert_literal(&r, false);

    // duals for `||`
    let r = eval(&mut ev, "principal.age + 1 > 18 || true", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18 || true", &[T, E]);
    let r = eval(&mut ev, "principal.active || true", &view()).await;
    assert_literal(&r, true);
    let mut ev = evaluator(&schema);

    // `x && true` is `x`, `x || false` is `x`
    let r = eval(&mut ev, "principal.age + 1 > 18 && true", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18", &[T, F, E]);
    let r = eval(&mut ev, "principal.age + 1 > 18 || false", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18", &[T, F, E]);
}

#[tokio::test]
async fn error_stops_evaluation() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let before = ev.solver_queries();
    // Integer overflow folds to an error at compile time, so the left operand
    // needs no solver query and the right operand must never be queried. The
    // satisfiability check of the (empty) assumptions and `check_equivalent`
    // fold to constants too (`User` has no `User` ancestors, so there is no
    // acyclicity constraint), so no query reaches the solver at all.
    let r = eval(
        &mut ev,
        "1 + 9223372036854775807 > 0 && principal.active",
        &view(),
    )
    .await;
    assert_eq!(r.data().outcomes(), &outcomes(&[E]));
    assert!(literal_of(&r).is_none());
    assert_eq!(ev.solver_queries() - before, 0);
}

#[tokio::test]
async fn trail_assumptions() {
    let schema = schema();
    let mut ev = evaluator(&schema);

    // The second disjunct is dead: `a && !a` under `!(a && b)`.
    let r = eval(
        &mut ev,
        "(principal.active && principal.age > 18) || (principal.active && !principal.active)",
        &view(),
    )
    .await;
    assert_residual(&r, "principal.active && principal.age > 18", &[T, F, E]);

    // `!a` is known under `!a`.
    let r = eval(
        &mut ev,
        "principal.active || (!principal.active && principal.age > 18)",
        &view(),
    )
    .await;
    assert_residual(&r, "principal.active || principal.age > 18", &[T, F, E]);
}

#[tokio::test]
async fn trail_gating() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("!principal.active"));

    // `principal.age + 1 > 18 && principal.active` is `{False, Error}`:
    // neither a literal nor a sure error, but it cannot be true, so the right
    // operand of the outer `&&` is dead and must not be evaluated under the
    // (unsatisfiable) assumption that the left one is true.
    let r = eval(
        &mut ev,
        "(principal.age + 1 > 18 && principal.active) && principal.age > 18",
        &view(),
    )
    .await;
    assert_residual(&r, "principal.age + 1 > 18 && false", &[F, E]);

    // Same for `if`: a branch is visited only if the test can reach it.
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(expr("principal.age + 1 > 18"), outcomes(&[T, E]));
    let r = eval(
        &mut ev,
        "if principal.age + 1 > 18 then principal.active else principal.age > 18",
        &view(),
    )
    .await;
    assert_residual(
        &r,
        "if principal.age + 1 > 18 then principal.active else principal.age > 18",
        &[T, F, E],
    );
    let ExprKind::If {
        test_expr,
        then_expr,
        else_expr,
    } = r.expr_kind()
    else {
        panic!("expected an `if`, got `{r}`");
    };
    assert_eq!(test_expr.data().outcomes(), &outcomes(&[T, E]));
    assert_eq!(then_expr.data().outcomes(), &outcomes(&[T, F]));
    assert_eq!(else_expr.data(), &EvaluationMetadata::default());
}

#[tokio::test]
async fn if_folding() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let r = eval(
        &mut ev,
        "if principal.active then principal.age > 18 else false",
        &view(),
    )
    .await;
    assert_residual(
        &r,
        "if principal.active then principal.age > 18 else false",
        &[T, F, E],
    );

    ev.assume_expr(expr("principal.active"));
    let r = eval(
        &mut ev,
        "if principal.active then principal.age > 18 else false",
        &view(),
    )
    .await;
    assert_residual(&r, "principal.age > 18", &[T, F]);
}

#[tokio::test]
async fn not_folding() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let r = eval(&mut ev, "!(principal.age + 1 > 18)", &view()).await;
    assert_residual(&r, "!(principal.age + 1 > 18)", &[T, F, E]);
    ev.assume_expr(expr("principal.active"));
    let r = eval(&mut ev, "!principal.active", &view()).await;
    assert_literal(&r, false);
}

#[tokio::test]
async fn assumed_outcomes() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(expr("principal.age + 1 > 18"), outcomes(&[F, E]));
    let r = eval(&mut ev, "principal.age + 1 > 18", &view()).await;
    assert_residual(&r, "principal.age + 1 > 18", &[F, E]);

    // `a > b` parses as `!(a <= b)`, so the `!` is a structure node whose
    // operand necessarily errors; like `&&`/`||`/`if`, `!` then collapses to
    // the erroring operand.
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(expr("principal.age + 1 > 18"), outcomes(&[E]));
    let r = eval(&mut ev, "principal.age + 1 > 18", &view()).await;
    assert_residual(&r, "principal.age + 1 <= 18", &[E]);
}

#[tokio::test]
async fn policyset_decision() {
    let schema = schema();
    let policy = parse_policy(
        None,
        "permit(principal, action, resource) when { principal.active };",
    )
    .unwrap();
    let pset = PolicySet::singleton(policy.into());

    let mut ev = evaluator(&schema);
    ev.assume_policyset_decision(pset.clone(), Decision::Allow);
    let r = eval(&mut ev, "principal.active", &view()).await;
    assert_literal(&r, true);

    let mut ev = evaluator(&schema);
    ev.assume_policyset_decision(pset, Decision::Deny);
    // denied because `principal.active` is false — or because it errored
    // on a missing principal
    let r = eval(&mut ev, "principal.active", &view()).await;
    assert_residual(&r, "principal.active", &[F, E]);
}

#[tokio::test]
async fn extra_assumptions_are_per_call() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    let e = expr("principal.active");
    let r = ev
        .evaluate(&e, &view(), vec![expr("principal.active")])
        .await
        .unwrap();
    assert_literal(&r, true);
    let r = ev.evaluate(&e, &view(), no_extra()).await.unwrap();
    assert_residual(&r, "principal.active", &[T, F, E]);
}

#[tokio::test]
async fn errors() {
    let schema = schema();

    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal.active"));
    ev.assume_expr(expr("!principal.active"));
    let err = ev
        .evaluate(&expr("principal.age > 18"), &view(), no_extra())
        .await
        .unwrap_err();
    assert!(
        matches!(err, EvaluationError::UnsatisfiableAssumptions),
        "{err}"
    );

    let mut ev = evaluator(&schema);
    let err = ev
        .evaluate(&expr("principal.nonexistent"), &view(), no_extra())
        .await
        .unwrap_err();
    assert!(matches!(err, EvaluationError::NotWellTyped { .. }), "{err}");

    // A non-boolean expression does not typecheck as a policy condition.
    let err = ev
        .evaluate(&expr("principal.age"), &view(), no_extra())
        .await
        .unwrap_err();
    assert!(matches!(err, EvaluationError::NotWellTyped { .. }), "{err}");

    let err = ev
        .evaluate(
            &expr("principal.active"),
            &req_env_from_strs("User", r#"Action::"nope""#, "Document"),
            no_extra(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EvaluationError::RequestEnvNotFound(_)),
        "{err}"
    );

    // An assumption that is ill-typed in the evaluated env is an error there.
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal.active"));
    let err = ev
        .evaluate(
            &expr("resource.protected"),
            &req_env_from_strs("Group", r#"Action::"admin""#, "Document"),
            no_extra(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EvaluationError::AssumptionNotWellTyped { .. }),
        "{err}"
    );
}

#[tokio::test]
async fn assumptions_apply_in_every_env() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal.active"));
    let r = eval(&mut ev, "principal.active && principal.age > 18", &view()).await;
    assert_residual(&r, "principal.age > 18", &[T, F]);
    let r = eval(&mut ev, "principal.active && principal.age > 18", &edit()).await;
    assert_residual(&r, "principal.age > 18", &[T, F]);
    // and one registered after an env was prepared is compiled into it lazily
    ev.assume_expr(expr("principal.age > 18"));
    let r = eval(&mut ev, "principal.active && principal.age > 18", &view()).await;
    assert_literal(&r, true);
}

#[tokio::test]
async fn hierarchy_is_enforced() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    // With a transitively closed, acyclic hierarchy, membership propagates.
    ev.assume_expr(expr(r#"principal in Group::"a""#));
    ev.assume_expr(expr(r#"Group::"a" in Group::"b""#));
    let r = eval(&mut ev, r#"principal in Group::"b""#, &view()).await;
    assert_literal(&r, true);
}

#[tokio::test]
async fn query_trace_records_all_queries() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal.active"));
    ev.record_trace(true);
    assert!(ev.take_trace().is_none(), "no trace before an evaluation");

    let e = expr("principal.active && principal.age > 18");
    let result = ev.evaluate(&e, &view(), no_extra()).await.unwrap();
    assert!(ev
        .check_equivalent(&e, &result, &view(), no_extra())
        .await
        .unwrap());

    let trace = ev.take_trace().unwrap();
    // The typechecker's wrapper leaves `true && (true && (true && e))`.
    assert_eq!(
        trace.erased,
        expr("true && (true && (true && (principal.active && principal.age > 18)))")
    );
    // The first query is the up-front satisfiability check of the base.
    let first = trace.queries.first().unwrap();
    assert_eq!(first.asserts, trace.base);
    assert!(!first.unsat);
    // `check_equivalent` contributed its base and exactly one final query.
    let ce_base = trace.ce_base.as_ref().unwrap();
    let last = trace.queries.last().unwrap();
    assert_eq!(last.asserts.len(), ce_base.len() + 1);
    assert_eq!(last.asserts.get(..ce_base.len()), Some(&ce_base[..]));
    assert!(last.unsat, "result must be equivalent");
    // Atom queries extend base ++ trail with one constraint each.
    for q in &trace.queries {
        assert_eq!(q.asserts.get(..trace.base.len()), Some(&trace.base[..]));
    }
    // Taking the trace clears it; the next evaluation starts a fresh one.
    assert!(ev.take_trace().is_none());
    ev.evaluate(&expr("principal.active"), &view(), no_extra())
        .await
        .unwrap();
    let trace = ev.take_trace().unwrap();
    assert!(trace.ce_base.is_none());
    assert_eq!(
        trace.erased,
        expr("true && (true && (true && principal.active))")
    );
}

/// `iferror(e, d)` coalesces `e`'s error into `d`: the `E` outcome
/// disappears (`iferror(e, false)` is "`e` is true" as a boolean), the
/// fallback contributes only where `e` can error, and every result is
/// solver-equivalent to its input (checked by `eval`).
#[tokio::test]
async fn iferror_coalesces_errors() {
    let schema = schema();
    let mut ev = evaluator(&schema);

    // `e` may overflow: {T, F, E} becomes {T, F}
    let r = eval(&mut ev, "iferror(principal.age + 1 > 18, false)", &view()).await;
    assert_residual(&r, "iferror(principal.age + 1 > 18, false)", &[T, F]);
    let r = eval(&mut ev, "!iferror(principal.age + 1 > 18, false)", &view()).await;
    assert_residual(&r, "!iferror(principal.age + 1 > 18, false)", &[T, F]);

    // an error-free `e` makes the fallback irrelevant, even an erroring one
    // (`principal.active || true` is true exactly when the principal exists:
    // its left operand is always evaluated)
    ev.assume_expr(expr("principal.active || true"));
    let r = eval(
        &mut ev,
        "iferror(principal.active, principal.age + 1 > 18)",
        &view(),
    )
    .await;
    assert_residual(
        &r,
        "iferror(principal.active, principal.age + 1 > 18)",
        &[T, F],
    );

    // a fallback that may itself error where `e` does keeps `E` reachable
    // (`age + 2` overflows wherever `age + 1` does)
    let r = eval(
        &mut ev,
        "iferror(principal.age + 1 > 18, principal.age + 2 > 0)",
        &view(),
    )
    .await;
    assert_residual(
        &r,
        "iferror(principal.age + 1 > 18, principal.age + 2 > 0)",
        &[T, F, E],
    );
    // ... but not one that cannot: `age - 1` underflows only at the minimum,
    // where `age + 1` does not overflow
    let r = eval(
        &mut ev,
        "iferror(principal.age + 1 > 18, principal.age - 1 < 0)",
        &view(),
    )
    .await;
    assert_residual(
        &r,
        "iferror(principal.age + 1 > 18, principal.age - 1 < 0)",
        &[T, F],
    );

    // literal arguments fold
    let r = eval(&mut ev, "iferror(true, false)", &view()).await;
    assert_literal(&r, true);

    // under an assumption that fixes `e`, the coalesced form folds with it
    ev.assume_expr(expr("principal.age == 20"));
    let r = eval(&mut ev, "iferror(principal.age + 1 > 18, false)", &view()).await;
    assert_literal(&r, true);
}

/// Plan 5: a `has`/`hasTag` guard of an optional attribute whose access
/// follows in the scope it guards is never folded to `true`, so the
/// residual validates on its own; a guard nothing follows, or whose scope
/// folded to a literal, folds as before.
#[tokio::test]
async fn kept_has_guards() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal has nick"));

    // true under the assumption, yet kept for its access, with outcomes {T}
    let r = eval(
        &mut ev,
        r#"principal has nick && principal.nick == "al""#,
        &view(),
    )
    .await;
    assert_residual(
        &r,
        r#"principal has nick && principal.nick == "al""#,
        &[T, F],
    );
    let ExprKind::And { left, .. } = r.expr_kind() else {
        panic!("expected an `&&`, got `{r}`");
    };
    assert_eq!(left.data().outcomes(), &outcomes(&[T]));

    // a required attribute needs no guard (its `has` is `true` for a present
    // entity, which the assumption proves): folded
    let r = eval(
        &mut ev,
        r#"principal has name && principal.name == "x""#,
        &view(),
    )
    .await;
    assert_residual(&r, r#"principal.name == "x""#, &[T, F]);

    // nothing follows that needs the guard: folded
    let r = eval(&mut ev, "principal has nick && principal.active", &view()).await;
    assert_residual(&r, "principal.active", &[T, F]);

    // the guarded scope folded to a literal: the guard goes with it
    let r = eval(
        &mut ev,
        "principal has nick && principal.nick == principal.nick",
        &view(),
    )
    .await;
    assert_literal(&r, true);

    // an `if` keeps its test (the else branch is dead but stays as written)
    let r = eval(
        &mut ev,
        r#"if principal has nick then principal.nick == "al" else false"#,
        &view(),
    )
    .await;
    assert_residual(
        &r,
        r#"if principal has nick then principal.nick == "al" else false"#,
        &[T, F],
    );
    let r = eval(
        &mut ev,
        "if principal has nick then principal.nick == principal.nick else false",
        &view(),
    )
    .await;
    assert_literal(&r, true);

    // not a guard: folded
    let r = eval(&mut ev, "!(principal has nick)", &view()).await;
    assert_literal(&r, false);

    // tags: every `getTag` needs its guard
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr(r#"principal.hasTag("t")"#));
    let r = eval(
        &mut ev,
        r#"principal.hasTag("t") && principal.getTag("t") > 1"#,
        &view(),
    )
    .await;
    assert_residual(
        &r,
        r#"principal.hasTag("t") && principal.getTag("t") > 1"#,
        &[T, F],
    );
    let r = eval(
        &mut ev,
        r#"principal.hasTag("t") && principal.age > 1"#,
        &view(),
    )
    .await;
    assert_residual(&r, "principal.age > 1", &[T, F]);
}
