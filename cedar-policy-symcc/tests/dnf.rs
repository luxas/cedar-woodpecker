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

//! Tests for the DNF converter.
//!
//! The expected DNFs below are derived by hand from the path construction
//! described in `src/dnf/mod.rs` and Cedar's three-valued, short-circuiting
//! semantics. They are not snapshots: if the converter changes behaviour, fix
//! the converter or argue the table is wrong. Every expected DNF is also
//! checked against the input in two independent ways: an exhaustive
//! three-valued truth table over the atoms (`interpret`), and the solver
//! (`Evaluator::check_equivalent`). A wrong expectation the converter happens
//! to reproduce is therefore caught as well.

#![expect(clippy::unwrap_used, clippy::panic, reason = "test code")]

mod utils;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::Arc;

use cedar_policy::{Expression, RequestEnv, Schema};
use cedar_policy_core::ast::{Expr, ExprBuilder, ExprKind, Literal, UnaryOp};
use cedar_policy_core::expr_builder::ExprBuilder as _;
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_symcc::dnf::{interpret, Dnf, DnfError, DEFAULT_MAX_CUBES};
use cedar_policy_symcc::evaluator::{with_default_metadata, EvaluationMetadata, Evaluator};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;
use nonempty_collections::{nes, NESet};

use utils::{req_env_from_strs, schema_from_cedarstr};

const SCHEMA: &str = r#"
entity Group in [Group];
entity User in [Group] {
    name: String, active: Bool, age: Long, nick?: String, flag?: Bool,
    tags?: Set<Long>, parent?: Group,
};
entity Document { protected: Bool, owner: User, level: Long };
action view appliesTo { principal: User, resource: Document };
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

fn evaluator(schema: &Schema) -> Evaluator<LocalSolver> {
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap()).unwrap();
    Evaluator::new(compiler, schema).unwrap()
}

use EvaluationOutcome::{Error as E, False as F, True as T};

/// The atoms the tables are written over. `a`, `b`, `c` cannot error; `e`
/// can (integer overflow).
const A: &str = "principal.active";
const B: &str = r#"principal.name == "x""#;
const C: &str = "resource.protected";
const ERR: &str = "principal.age + 1 == 19";

/// Substitutes the atom names of a table entry: a standalone `a`, `b`, `c`
/// or `e` becomes the parenthesised atom.
fn t(template: &str) -> Expr {
    let chars: Vec<char> = template.chars().collect();
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let mut out = String::new();
    for (i, ch) in chars.iter().enumerate() {
        let standalone = !chars.get(i.wrapping_sub(1)).is_some_and(is_word)
            && !chars.get(i + 1).is_some_and(is_word);
        let atom = match ch {
            'a' if standalone => Some(A),
            'b' if standalone => Some(B),
            'c' if standalone => Some(C),
            'e' if standalone => Some(ERR),
            _ => None,
        };
        match atom {
            Some(atom) => write!(out, "({atom})").unwrap(),
            None => out.push(*ch),
        }
    }
    expr(&out)
}

// ---------------------------------------------------------------------------
// Three-valued truth-table oracle

/// The atoms of `e` (its non-structure nodes), in order of first occurrence.
fn atoms_of(e: &Expr, out: &mut Vec<Expr>) {
    match e.expr_kind() {
        ExprKind::Lit(Literal::Bool(_)) => {}
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => atoms_of(arg, out),
        ExprKind::And { left, right } | ExprKind::Or { left, right } => {
            atoms_of(left, out);
            atoms_of(right, out);
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => {
            atoms_of(test_expr, out);
            atoms_of(then_expr, out);
            atoms_of(else_expr, out);
        }
        _ => {
            if !out.contains(e) {
                out.push(e.clone());
            }
        }
    }
}

/// Every assignment of `outcomes` to `atoms`.
fn assignments(
    atoms: &[Expr],
    outcomes: &[EvaluationOutcome],
) -> Vec<HashMap<Expr, EvaluationOutcome>> {
    let mut all = vec![HashMap::new()];
    for atom in atoms {
        all = all
            .into_iter()
            .flat_map(|partial| {
                outcomes.iter().map(move |o| {
                    let mut next = partial.clone();
                    next.insert(atom.clone(), o.clone());
                    next
                })
            })
            .collect();
    }
    all
}

fn interpret_under(e: &Expr, assignment: &HashMap<Expr, EvaluationOutcome>) -> EvaluationOutcome {
    interpret(e, |atom| {
        assignment
            .get(atom)
            .unwrap_or_else(|| panic!("unassigned atom `{atom}`"))
            .clone()
    })
}

/// Asserts that `original` and `rewritten` agree on every assignment of
/// `outcomes` to the atoms of `original`.
#[track_caller]
fn assert_truth_table(original: &Expr, rewritten: &Expr, outcomes: &[EvaluationOutcome]) {
    let mut atoms = Vec::new();
    atoms_of(original, &mut atoms);
    for assignment in assignments(&atoms, outcomes) {
        assert_eq!(
            interpret_under(original, &assignment),
            interpret_under(rewritten, &assignment),
            "`{original}` and `{rewritten}` differ under {assignment:?}"
        );
    }
}

/// Whether `e` is an `||` chain of `&&` chains of literals (an atom, `!atom`,
/// or `false` as the last conjunct).
fn is_dnf(e: &Expr) -> bool {
    fn is_atom(e: &Expr) -> bool {
        !matches!(
            e.expr_kind(),
            ExprKind::Lit(Literal::Bool(_))
                | ExprKind::And { .. }
                | ExprKind::Or { .. }
                | ExprKind::If { .. }
                | ExprKind::UnaryApp {
                    op: UnaryOp::Not,
                    ..
                }
        )
    }
    fn is_literal(e: &Expr) -> bool {
        match e.expr_kind() {
            ExprKind::UnaryApp {
                op: UnaryOp::Not,
                arg,
            } => is_atom(arg),
            _ => is_atom(e),
        }
    }
    fn is_cube(e: &Expr) -> bool {
        match e.expr_kind() {
            ExprKind::Lit(Literal::Bool(_)) => true,
            ExprKind::And { left, right } => {
                is_cube_prefix(left)
                    && (is_literal(right)
                        || matches!(right.expr_kind(), ExprKind::Lit(Literal::Bool(false))))
            }
            _ => is_literal(e),
        }
    }
    fn is_cube_prefix(e: &Expr) -> bool {
        match e.expr_kind() {
            ExprKind::And { left, right } => is_cube_prefix(left) && is_literal(right),
            _ => is_literal(e),
        }
    }
    match e.expr_kind() {
        ExprKind::Or { left, right } => is_dnf(left) && is_cube(right),
        _ => is_cube(e),
    }
}

// ---------------------------------------------------------------------------
// Rule tables

/// Converts `input`, checks the result is structurally `expected`, and that it
/// agrees with the input on every three-valued assignment of the atoms.
#[track_caller]
fn assert_dnf(input: &Expr, expected: &Expr) -> Dnf<()> {
    let dnf = Dnf::of_expr(input).unwrap();
    assert_eq!(
        dnf.to_expr(),
        *expected,
        "expected DNF `{expected}` for `{input}`, got `{dnf}`"
    );
    assert!(is_dnf(&dnf.to_expr()), "`{dnf}` is not in normal form");
    assert_truth_table(input, &dnf.to_expr(), &[T, F, E]);
    dnf
}

/// The hand-derived rewrite table: (input, DNF).
fn rule_table() -> Vec<(&'static str, &'static str)> {
    vec![
        // the README's motivating rewrite: `b` is only reached when `a` is false
        ("(a || b) && c", "(a && c) || (!a && b && c)"),
        // right distribution carries the same guard: `c` only when `b` is false
        ("a && (b || c)", "(a && b) || (a && !b && c)"),
        // De Morgan, three-valued: `!b` only after `a` was true
        ("!(a && b)", "(a && !b) || !a"),
        ("!(a || b)", "!a && !b"),
        ("if a then b else c", "(a && b) || (!a && c)"),
        ("!!a", "a"),
        ("a && true", "a"),
        ("a || false", "a"),
        ("true && a", "a"),
        ("false || a", "a"),
        // `true` grafted under the `!a` edge
        ("a || true", "a || !a"),
        ("true", "true"),
        ("false", "false"),
        // never true, but `a` may error: the cube is kept to reproduce that
        ("a && false", "a && false"),
        ("false && a", "false"),
        // a repeated atom is determined by the prefix
        ("a || a", "a"),
        ("a && (b && a)", "a && b"),
        ("(a && b) || (a && !a)", "a && b"),
        ("a || (!a && b)", "a || (!a && b)"),
        // the erroring atom is on a true cube: no extra cube needed
        ("(e || b) && c", "(e && c) || (!e && b && c)"),
        // nested structure
        (
            "(a || b) && (c || e)",
            "(a && c) || (a && !c && e) || (!a && b && c) || (!a && b && !c && e)",
        ),
        (
            "if (a && b) then c else e",
            "(a && b && c) || (a && !b && e) || (!a && e)",
        ),
    ]
}

#[test]
fn rule_tables() {
    for (input, expected) in rule_table() {
        assert_dnf(&t(input), &t(expected));
    }
}

#[test]
fn comparison_desugaring_is_a_negated_literal() {
    // `x > 1` parses as `!(x <= 1)`, so it is the negation of the atom `x <= 1`.
    let dnf = assert_dnf(&expr("principal.age > 1"), &expr("!(principal.age <= 1)"));
    let [cube] = dnf.cubes() else {
        panic!("expected one cube, got `{dnf}`");
    };
    let [lit] = cube.literals() else {
        panic!("expected one literal, got `{cube}`");
    };
    assert!(lit.negated());
    assert_eq!(**lit.atom(), expr("principal.age <= 1"));
}

#[test]
fn never_true_cubes_are_only_kept_for_errors() {
    // With no atom able to error, no never-true cube survives, and the
    // result agrees with the input wherever no atom errors.
    for (input, _) in rule_table() {
        let input = t(input);
        let dnf = Dnf::of(&input, |_| false, DEFAULT_MAX_CUBES).unwrap();
        assert!(
            dnf.cubes().iter().all(|c| !c.is_never_true()),
            "never-true cube in `{dnf}` for `{input}`"
        );
        assert_eq!(dnf.true_cubes().count(), dnf.cubes().len());
        assert_truth_table(&input, &dnf.to_expr(), &[T, F]);
    }
    // Only cubes with an uncovered erroring node are kept: `e` is evaluated
    // both after `a` and after `!a`, and those are distinct nodes of the tree.
    let dnf = Dnf::of(
        &t("(a && false) || (e && false)"),
        |atom| *atom == t("e"),
        DEFAULT_MAX_CUBES,
    )
    .unwrap();
    assert_eq!(
        dnf.to_expr(),
        t("(a && e && false) || (!a && e && false)"),
        "got `{dnf}`"
    );
}

// ---------------------------------------------------------------------------
// Exhaustive check over small expression shapes

/// Every boolean-structure expression with exactly `size` nodes over `leaves`.
fn shapes(size: usize, leaves: &[Expr]) -> Vec<Vec<Expr>> {
    let node = |kind| ExprBuilder::new().with_expr_kind(kind);
    let mut by_size: Vec<Vec<Expr>> = vec![vec![], leaves.to_vec()];
    for n in 2..=size {
        let mut out = Vec::new();
        for x in by_size.get(n - 1).unwrap() {
            out.push(node(ExprKind::UnaryApp {
                op: UnaryOp::Not,
                arg: Arc::new(x.clone()),
            }));
        }
        for i in 1..n - 1 {
            let j = n - 1 - i;
            for l in by_size.get(i).unwrap() {
                for r in by_size.get(j).unwrap() {
                    out.push(node(ExprKind::And {
                        left: Arc::new(l.clone()),
                        right: Arc::new(r.clone()),
                    }));
                    out.push(node(ExprKind::Or {
                        left: Arc::new(l.clone()),
                        right: Arc::new(r.clone()),
                    }));
                }
            }
        }
        for i in 1..n {
            for j in 1..n - i {
                let k = n - 1 - i - j;
                if k == 0 {
                    continue;
                }
                for c in by_size.get(i).unwrap() {
                    for a in by_size.get(j).unwrap() {
                        for b in by_size.get(k).unwrap() {
                            out.push(node(ExprKind::If {
                                test_expr: Arc::new(c.clone()),
                                then_expr: Arc::new(a.clone()),
                                else_expr: Arc::new(b.clone()),
                            }));
                        }
                    }
                }
            }
        }
        by_size.push(out);
    }
    by_size
}

#[test]
fn exhaustive_small_shapes() {
    let leaves = [t("a"), t("b"), t("c"), t("true"), t("false")];
    let by_size = shapes(6, &leaves);
    let mut checked = 0;
    for exprs in &by_size {
        for e in exprs {
            let dnf = Dnf::of_expr(e).unwrap();
            let rewritten = dnf.to_expr();
            assert!(
                is_dnf(&rewritten),
                "`{rewritten}` (from `{e}`) is not in normal form"
            );
            assert_truth_table(e, &rewritten, &[T, F, E]);
            let error_free = Dnf::of(e, |_| false, DEFAULT_MAX_CUBES).unwrap();
            assert!(error_free.cubes().iter().all(|c| !c.is_never_true()));
            assert_truth_table(e, &error_free.to_expr(), &[T, F]);
            checked += 1;
        }
    }
    assert!(checked > 10_000, "only {checked} shapes checked");
}

// ---------------------------------------------------------------------------
// Solver checks (the evaluator as an oracle)

fn no_extra() -> Vec<Expr> {
    Vec::new()
}

/// Asserts that `dnf` is solver-equivalent to `original` under `ev`'s assumptions.
async fn assert_equivalent<T: Sync>(
    ev: &mut Evaluator<LocalSolver>,
    original: &Expr,
    dnf: &Dnf<T>,
) {
    let rewritten = with_default_metadata(&dnf.to_expr()).unwrap();
    assert!(
        ev.check_equivalent(original, &rewritten, &view(), no_extra())
            .await
            .unwrap(),
        "`{dnf}` is not solver-equivalent to `{original}`"
    );
}

#[tokio::test]
async fn rule_tables_are_solver_equivalent() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    for (input, _) in rule_table() {
        let input = t(input);
        let dnf = Dnf::of_expr(&input).unwrap();
        assert_equivalent(&mut ev, &input, &dnf).await;
    }
}

/// `can_error` taken from the evaluator's outcome sets.
fn can_error_by_metadata(atom: &Expr<EvaluationMetadata>) -> bool {
    !atom.data().is_error_free()
}

fn outcomes(set: &[EvaluationOutcome]) -> NESet<EvaluationOutcome> {
    let mut iter = set.iter().cloned();
    let mut res = nes![iter.next().unwrap()];
    for o in iter {
        res.insert(o);
    }
    res
}

#[tokio::test]
async fn evaluator_metadata_drops_error_cubes() {
    let schema = schema();
    // `if e then false else false` keeps its structure through the evaluator
    // (only atoms fold), so the converter sees `e` with its outcome set.
    let input = t("if e then false else false");

    // Without assumptions `e` may overflow: the never-true cube stays.
    let mut ev = evaluator(&schema);
    let evaluated = ev.evaluate(&input, &view(), no_extra()).await.unwrap();
    let dnf = Dnf::of(&evaluated, can_error_by_metadata, DEFAULT_MAX_CUBES).unwrap();
    assert_eq!(dnf.to_expr(), t("e && false"), "got `{dnf}`");
    assert_equivalent(&mut ev, &input, &dnf).await;

    // Assumed error-free, the cube has nothing to reproduce.
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(t("e"), outcomes(&[T, F]));
    let evaluated = ev.evaluate(&input, &view(), no_extra()).await.unwrap();
    let dnf = Dnf::of(&evaluated, can_error_by_metadata, DEFAULT_MAX_CUBES).unwrap();
    assert_eq!(dnf.to_expr(), t("false"), "got `{dnf}`");
    assert!(dnf.cubes().is_empty());
    assert_equivalent(&mut ev, &input, &dnf).await;
}

#[tokio::test]
async fn unvisited_branches_keep_their_error_cubes() {
    let schema = schema();
    let input = t("if e then a else (c && false)");

    // `e` cannot be false: the else branch is never visited and keeps the
    // default metadata, so `c` counts as able to error and its cube stays.
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(t("e"), outcomes(&[T, E]));
    let evaluated = ev.evaluate(&input, &view(), no_extra()).await.unwrap();
    let dnf = Dnf::of(&evaluated, can_error_by_metadata, DEFAULT_MAX_CUBES).unwrap();
    assert_eq!(
        dnf.to_expr(),
        t("(e && a) || (!e && c && false)"),
        "got `{dnf}`"
    );
    assert_equivalent(&mut ev, &input, &dnf).await;

    // Visited, `c && false` folds to `false` before the converter sees it
    // (once the resource is known to exist: `c || true` is true exactly when
    // its left operand evaluates).
    let mut ev = evaluator(&schema);
    ev.assume_expr_outcomes(t("e"), outcomes(&[T, F]));
    ev.assume_expr(t("c || true"));
    let evaluated = ev.evaluate(&input, &view(), no_extra()).await.unwrap();
    let dnf = Dnf::of(&evaluated, can_error_by_metadata, DEFAULT_MAX_CUBES).unwrap();
    assert_eq!(dnf.to_expr(), t("e && a"), "got `{dnf}`");
    assert_equivalent(&mut ev, &input, &dnf).await;
}

// ---------------------------------------------------------------------------
// Limits

fn clauses(n: usize) -> Expr {
    let clause = |i: usize| format!("(principal.age == {i} || resource.level == {i})");
    expr(&(0..n).map(clause).collect::<Vec<_>>().join(" && "))
}

#[test]
fn cube_budget() {
    assert_eq!(
        Dnf::of_expr(&clauses(13)).unwrap_err(),
        DnfError::TooLarge {
            limit: DEFAULT_MAX_CUBES,
            what: "cubes"
        }
    );
    let dnf = Dnf::of(&clauses(3), |_| true, usize::MAX).unwrap();
    assert_eq!(dnf.true_cubes().count(), 8);
    assert_truth_table(&clauses(3), &dnf.to_expr(), &[T, F, E]);
}
