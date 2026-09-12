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

// ---------------------------------------------------------------------------
// Step 2: splitting atoms

use cedar_policy_symcc::dnf::{split_atoms, DEFAULT_MAX_SPLIT_NODES};

/// Whether `e` is a boolean structure over clean atoms: no atom contains any
/// `&&`/`||`/`!`/`if` node.
fn atoms_are_clean(e: &Expr) -> bool {
    fn is_offender(e: &Expr) -> bool {
        matches!(
            e.expr_kind(),
            ExprKind::And { .. }
                | ExprKind::Or { .. }
                | ExprKind::If { .. }
                | ExprKind::UnaryApp {
                    op: UnaryOp::Not,
                    ..
                }
        )
    }
    match e.expr_kind() {
        ExprKind::Lit(Literal::Bool(_)) => true,
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => atoms_are_clean(arg),
        ExprKind::And { left, right } | ExprKind::Or { left, right } => {
            atoms_are_clean(left) && atoms_are_clean(right)
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => atoms_are_clean(test_expr) && atoms_are_clean(then_expr) && atoms_are_clean(else_expr),
        _ => e
            .subexpressions()
            .skip(1)
            .all(|s| !is_offender(s) || under_iferror(e, s)),
    }
}

/// Whether `node` lies inside an `iferror(…)` call of `e` (which the splitter
/// never looks into, see `split_atoms`).
fn under_iferror(e: &Expr, node: &Expr) -> bool {
    e.subexpressions()
        .filter(|s| {
            matches!(s.expr_kind(), ExprKind::ExtensionFunctionApp { fn_name, .. }
                if fn_name.to_string() == "iferror")
        })
        .any(|call| call.subexpressions().skip(1).any(|s| std::ptr::eq(s, node)))
}

/// Splits `input`, asserts the exact result, that its atoms are clean, and
/// that its DNF is `expected_dnf`; returns the split expression.
#[track_caller]
fn assert_split(input: &Expr, expected: &Expr, expected_dnf: &Expr) -> Expr {
    let split = split_atoms(input, DEFAULT_MAX_SPLIT_NODES).unwrap();
    assert_eq!(
        split, *expected,
        "expected split `{expected}` for `{input}`, got `{split}`"
    );
    assert!(atoms_are_clean(&split), "`{split}` has unclean atoms");
    let dnf = Dnf::of_expr(&split).unwrap();
    assert_eq!(
        dnf.to_expr(),
        *expected_dnf,
        "expected DNF `{expected_dnf}` for split `{split}`, got `{dnf}`"
    );
    assert!(atoms_are_clean(&dnf.to_expr()));
    split
}

/// The split table: (input, split form, DNF of the split form). Atom names as
/// in `t()`; the examples use error-free atoms so the DNFs stay small.
fn split_table() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // the README's worked example, adapted to the test schema
        (
            r#"a && (if b && c then Document::"d1" else Document::"d2").level == 1"#,
            r#"a && (if b && c then Document::"d1".level == 1 else Document::"d2".level == 1)"#,
            r#"(a && b && c && Document::"d1".level == 1)
               || (a && b && !c && Document::"d2".level == 1)
               || (a && !b && Document::"d2".level == 1)"#,
        ),
        // an `if` under `==`; both branches fold (`1 == 3`, `2 == 3` are literal equalities)
        (
            "(if c then 1 else 2) == 3",
            "if c then false else false",
            "c && false",
        ),
        // boolean structure under `==`, other operand left alone
        (
            "(a && c) == principal.active",
            "if a && c then true == principal.active else false == principal.active",
            "(a && c && true == principal.active)
             || (a && !c && false == principal.active)
             || (!a && false == principal.active)",
        ),
        // the README's `x == y ⇒ (x && y) || (!x && !y)`, derived from hoisting + folding
        (
            "(a && c) == (b || principal.age == 1)",
            "if a && c then (if b || principal.age == 1 then true else false)
             else (if b || principal.age == 1 then false else true)",
            "(a && c && b) || (a && c && !b && principal.age == 1)
             || (a && !c && !b && !(principal.age == 1))
             || (!a && !b && !(principal.age == 1))",
        ),
        (
            "!a == b",
            "if !a then true == b else false == b",
            "(a && false == b) || (!a && true == b)",
        ),
        // an `if` operand of `==` is hoisted like any other (it may be of
        // any type); the equalities survive
        (
            "(if c then a else b) == principal.active",
            "if c then a == principal.active else b == principal.active",
            "(c && a == principal.active) || (!c && b == principal.active)",
        ),
        // deeper structure inside an `==` operand is hoisted too
        (
            "[a && c].contains(b) == principal.active",
            "if a && c then [true].contains(b) == principal.active
             else [false].contains(b) == principal.active",
            "(a && c && [true].contains(b) == principal.active)
             || (a && !c && [false].contains(b) == principal.active)
             || (!a && [false].contains(b) == principal.active)",
        ),
        // boolean structure inside a set literal — the case `==` alone misses
        (
            "[a && c].contains(b)",
            "if a && c then [true].contains(b) else [false].contains(b)",
            "(a && c && [true].contains(b)) || (a && !c && [false].contains(b))
             || (!a && [false].contains(b))",
        ),
        // a hoisted test that is itself an `if`, staying at structure position
        (
            "(if (if c then a else false) then 1 else 2) == 1",
            "if (if c then a else false) then true else false",
            "c && a",
        ),
        // clean atoms come back unchanged
        (
            "a && (b || !(principal.age == 1))",
            "a && (b || !(principal.age == 1))",
            "(a && b) || (a && !b && !(principal.age == 1))",
        ),
        // a left sibling: `principal.age` is evaluated before the hoisted
        // test, so its error must surface first — the guard `age == age`
        (
            "principal.age == (if c then 1 else 2)",
            "if principal.age == principal.age
             then (if c then principal.age == 1 else principal.age == 2) else false",
            "(principal.age == principal.age && c && principal.age == 1)
             || (principal.age == principal.age && !c && principal.age == 2)",
        ),
        // left siblings at two levels, outer first: `level`, then `age`
        (
            "resource.level < principal.age + (if c then 1 else 2)",
            "if resource.level == resource.level && principal.age == principal.age
             then (if c then resource.level < principal.age + 1
                   else resource.level < principal.age + 2) else false",
            "(resource.level == resource.level && principal.age == principal.age && c
              && resource.level < principal.age + 1)
             || (resource.level == resource.level && principal.age == principal.age && !c
              && resource.level < principal.age + 2)",
        ),
        // literal and variable siblings never err and get no guard
        (
            "[1, principal.age, if c then 2 else 3].contains(resource.level)",
            "if principal.age == principal.age
             then (if c then [1, principal.age, 2].contains(resource.level)
                   else [1, principal.age, 3].contains(resource.level)) else false",
            "(principal.age == principal.age && c && [1, principal.age, 2].contains(resource.level))
             || (principal.age == principal.age && !c
              && [1, principal.age, 3].contains(resource.level))",
        ),
        (
            "[principal, resource.owner, if c then principal else resource.owner].contains(principal)",
            "if resource.owner == resource.owner
             then (if c then [principal, resource.owner, principal].contains(principal)
                   else [principal, resource.owner, resource.owner].contains(principal)) else false",
            "(resource.owner == resource.owner && c
              && [principal, resource.owner, principal].contains(principal))
             || (resource.owner == resource.owner && !c
              && [principal, resource.owner, resource.owner].contains(principal))",
        ),
        // a left sibling that never errs by itself (`==`) contributes its
        // operands' guards, not its own
        (
            "(principal.age == 1) == (if a && b then true else false)",
            "if principal.age == principal.age
             then (if a && b then (principal.age == 1) == true else (principal.age == 1) == false)
             else false",
            "(principal.age == principal.age && a && b && (principal.age == 1) == true)
             || (principal.age == principal.age && a && !b && (principal.age == 1) == false)
             || (principal.age == principal.age && !a && (principal.age == 1) == false)",
        ),
        // an `if` operand on the right: the left operand is guarded
        (
            "resource.protected == (if a && b then true else false)",
            "if resource.protected == resource.protected
             then (if a && b then resource.protected == true else resource.protected == false)
             else false",
            "(resource.protected == resource.protected && a && b && resource.protected == true)
             || (resource.protected == resource.protected && a && !b
              && resource.protected == false)
             || (resource.protected == resource.protected && !a && resource.protected == false)",
        ),
        // an opaque `iferror` call as the left sibling is guarded like any atom
        (
            "iferror(e, false) == (if a && b then true else false)",
            "if iferror(e, false) == iferror(e, false)
             then (if a && b then iferror(e, false) == true else iferror(e, false) == false)
             else false",
            "(iferror(e, false) == iferror(e, false) && a && b && iferror(e, false) == true)
             || (iferror(e, false) == iferror(e, false) && a && !b
              && iferror(e, false) == false)
             || (iferror(e, false) == iferror(e, false) && !a && iferror(e, false) == false)",
        ),
        // the split of a substituted atom re-derives the guard; the DNF's
        // literal dedup keeps it once per cube
        (
            "principal.age == (if c then (if a then 1 else 2) else 3)",
            "if principal.age == principal.age
             then (if c then (if a then principal.age == 1 else principal.age == 2)
                   else principal.age == 3)
             else false",
            "(principal.age == principal.age && c && a && principal.age == 1)
             || (principal.age == principal.age && c && !a && principal.age == 2)
             || (principal.age == principal.age && !c && principal.age == 3)",
        ),
        // siblings that cannot err get no guard: a set literal of literals …
        (
            r#"["admin", "root"].contains(if c then principal.name else "guest")"#,
            r#"if c then ["admin", "root"].contains(principal.name)
               else ["admin", "root"].contains("guest")"#,
            r#"(c && ["admin", "root"].contains(principal.name))
               || (!c && ["admin", "root"].contains("guest"))"#,
        ),
        // … and a literal equality an earlier substitution produced (`1 == 1`)
        (
            "((if c then 1 else 2) == 1) == (if b || a then true else false)",
            "if c then (if b || a then true else false) else (if b || a then false else true)",
            "(c && b) || (c && !b && a) || (!c && !b && !a)",
        ),
        // a set or record literal sibling never errs by itself: its elements
        // are the guards
        (
            "[principal.age, 7].containsAll(if c then [1] else [2])",
            "if principal.age == principal.age
             then (if c then [principal.age, 7].containsAll([1])
                   else [principal.age, 7].containsAll([2])) else false",
            "(principal.age == principal.age && c && [principal.age, 7].containsAll([1]))
             || (principal.age == principal.age && !c && [principal.age, 7].containsAll([2]))",
        ),
        // a guard established by the left operand of `&&` (or the test of an
        // `if`) is known inside the right operand (the then branch)
        (
            "principal.age == principal.age && principal.age == (if c then 1 else 2)",
            "principal.age == principal.age
             && (if c then principal.age == 1 else principal.age == 2)",
            "(principal.age == principal.age && c && principal.age == 1)
             || (principal.age == principal.age && !c && principal.age == 2)",
        ),
        (
            "if principal.age == principal.age then principal.age == (if c then 1 else 2) else b",
            "if principal.age == principal.age
             then (if c then principal.age == 1 else principal.age == 2) else b",
            "(principal.age == principal.age && c && principal.age == 1)
             || (principal.age == principal.age && !c && principal.age == 2)
             || (!(principal.age == principal.age) && b)",
        ),
    ]
}

fn normalize(text: &str) -> Expr {
    t(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[test]
fn split_tables() {
    for (input, split, dnf) in split_table() {
        assert_split(&normalize(input), &normalize(split), &normalize(dnf));
    }
}

#[tokio::test]
async fn split_tables_are_solver_equivalent() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    for (input, _, _) in split_table() {
        let input = normalize(input);
        let split = split_atoms(&input, DEFAULT_MAX_SPLIT_NODES).unwrap();
        let annotated = with_default_metadata(&split).unwrap();
        assert!(
            ev.check_equivalent(&input, &annotated, &view(), no_extra())
                .await
                .unwrap(),
            "split `{split}` is not solver-equivalent to `{input}`"
        );
        let dnf = Dnf::of_expr(&split).unwrap();
        assert_equivalent(&mut ev, &input, &dnf).await;
    }
}

#[tokio::test]
async fn split_evaluate_dnf_pipeline() {
    // The README's "use the symbolic evaluator to guide the splitting",
    // decoupled: split first, evaluate the split expression (dead branches
    // fold, atoms get exact outcome sets), then convert with the metadata.
    let schema = schema();
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("resource.protected"));
    let (input, _, _) = *split_table().first().unwrap(); // c is `resource.protected`
    let input = normalize(input);
    let split = split_atoms(&input, DEFAULT_MAX_SPLIT_NODES).unwrap();
    let evaluated = ev.evaluate(&split, &view(), no_extra()).await.unwrap();
    let dnf = Dnf::of(&evaluated, can_error_by_metadata, DEFAULT_MAX_CUBES).unwrap();
    // Of the three cubes, the `b && !c` one is dead under the assumption.
    assert_eq!(dnf.true_cubes().count(), 2, "got `{dnf}`");
    assert_eq!(
        dnf.to_expr(),
        normalize(
            r#"(a && b && Document::"d1".level == 1) || (a && !b && Document::"d2".level == 1)"#
        ),
        "got `{dnf}`"
    );
    let annotated = with_default_metadata(&dnf.to_expr()).unwrap();
    assert!(ev
        .check_equivalent(&input, &annotated, &view(), no_extra())
        .await
        .unwrap());
}

#[test]
fn split_budget() {
    let (input, _, _) = *split_table().first().unwrap();
    let input = normalize(input);
    assert_eq!(
        split_atoms(&input, 10).unwrap_err(),
        DnfError::TooLarge {
            limit: 10,
            what: "atom nodes"
        }
    );
    split_atoms(&input, DEFAULT_MAX_SPLIT_NODES).unwrap();
}

#[test]
fn split_generated_sweep() {
    // Embed every small boolean structure S in two atom templates and check
    // the split against the structural reference semantics of the template:
    //   (if S then 1 else 2) == 1        ~ interpret(S)   (1 == 1 / 2 == 1 fold)
    //   principal.age == (if S then 1 else 2)
    //                                    ~ S=T ⇒ atom `age == 1`, S=F ⇒ `age == 2`
    let leaves = [t("a"), t("c"), t("true"), t("false")];
    let by_size = shapes(5, &leaves);
    let eq1 = expr("principal.age == 1");
    let eq2 = expr("principal.age == 2");
    let guard = expr("principal.age == principal.age");
    let mut checked = 0;
    for exprs in &by_size {
        for s in exprs {
            let node = |kind| ExprBuilder::new().with_expr_kind(kind);
            let ite = node(ExprKind::If {
                test_expr: Arc::new(s.clone()),
                then_expr: Arc::new(expr("1")),
                else_expr: Arc::new(expr("2")),
            });
            let t1 = node(ExprKind::BinaryApp {
                op: cedar_policy_core::ast::BinaryOp::Eq,
                arg1: Arc::new(ite.clone()),
                arg2: Arc::new(expr("1")),
            });
            let t2 = node(ExprKind::BinaryApp {
                op: cedar_policy_core::ast::BinaryOp::Eq,
                arg1: Arc::new(expr("principal.age")),
                arg2: Arc::new(ite),
            });
            let split1 = split_atoms(&t1, DEFAULT_MAX_SPLIT_NODES).unwrap();
            let split2 = split_atoms(&t2, DEFAULT_MAX_SPLIT_NODES).unwrap();
            assert!(atoms_are_clean(&split1), "unclean `{split1}` from `{t1}`");
            assert!(atoms_are_clean(&split2), "unclean `{split2}` from `{t2}`");
            // `principal.age` is a left sibling of the `if` in `t2`, so its
            // split is guarded by `age == age` — an atom that is never false.
            let atoms = vec![t("a"), t("c"), eq1.clone(), eq2.clone(), guard.clone()];
            for assignment in assignments(&atoms, &[T, F, E]) {
                if assignment.get(&guard) == Some(&F) {
                    continue;
                }
                let s_outcome = interpret_under(s, &assignment);
                // (if S then 1 else 2) == 1 evaluates like S itself.
                assert_eq!(
                    interpret_under(&split1, &assignment),
                    s_outcome,
                    "`{split1}` vs `{s}`"
                );
                // principal.age == (if S then 1 else 2) errs where `age` does,
                // and otherwise picks one equality atom.
                let expected = match (assignment.get(&guard).unwrap(), &s_outcome) {
                    (E, _) | (_, E) => EvaluationOutcome::Error,
                    (_, T) => assignment.get(&eq1).unwrap().clone(),
                    (_, F) => assignment.get(&eq2).unwrap().clone(),
                };
                assert_eq!(
                    interpret_under(&split2, &assignment),
                    expected,
                    "`{split2}` vs `{t2}`"
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 800, "only {checked} shapes checked");
}
// ---------------------------------------------------------------------------
// Phase 4, Step 1: eliminating record and set literals

use cedar_policy_symcc::dnf::{eliminate_aggregates, normalize_atoms};

/// Whether `e` is free of the literals [`eliminate_aggregates`] removes:
/// no record literal under `.attr`, `has` or `==` (both sides), no set
/// literal under `contains` (left), `containsAll` (right), `containsAny`
/// (either side), `==` (either side), `isEmpty` or `in` (right) — outside
/// opaque atoms.
fn eliminated(e: &Expr) -> bool {
    e.subexpressions().all(|s| {
        // inside an opaque atom (an `iferror` call) the second split does
        // not look, and neither does this
        under_iferror(e, s) || {
            let is_record = |x: &Expr| matches!(x.expr_kind(), ExprKind::Record(_));
            let is_set = |x: &Expr| matches!(x.expr_kind(), ExprKind::Set(_));
            match s.expr_kind() {
                ExprKind::GetAttr { expr, .. } | ExprKind::HasAttr { expr, .. } => !is_record(expr),
                ExprKind::UnaryApp {
                    op: UnaryOp::IsEmpty,
                    arg,
                } => !is_set(arg),
                ExprKind::BinaryApp { op, arg1, arg2 } => match op {
                    cedar_policy_core::ast::BinaryOp::Eq => {
                        !(is_record(arg1) && is_record(arg2)) && !is_set(arg1) && !is_set(arg2)
                    }
                    cedar_policy_core::ast::BinaryOp::Contains => !is_set(arg1),
                    cedar_policy_core::ast::BinaryOp::ContainsAll
                    | cedar_policy_core::ast::BinaryOp::In => !is_set(arg2),
                    cedar_policy_core::ast::BinaryOp::ContainsAny => !is_set(arg1) && !is_set(arg2),
                    _ => true,
                },
                _ => true,
            }
        }
    })
}

/// The elimination table: (input, normalized form). Every row is checked
/// exactly, for elimination-freedom and cleanliness, and against the input
/// with the solver.
fn elim_table() -> Vec<(&'static str, &'static str)> {
    vec![
        // `.attr` on a record literal: the field, guarded by every field
        (
            "{x: principal.age, y: principal.name}.x == 3",
            "if principal.age == principal.age && principal.name == principal.name
             then principal.age == 3 else false",
        ),
        // nested records; a guard the term evaluates first anyway is dropped
        ("{x: {y: principal.age}}.x.y == 1", "principal.age == 1"),
        // `has` on a record literal is decided; a literal field needs no guard
        ("{x: 1} has x", "true"),
        ("{x: 1} has y || a", "false || a"),
        // record equality, key by key
        (
            "{x: principal.age, y: 1} == {x: 3, y: 1}",
            "principal.age == 3 && true",
        ),
        // a set literal's `contains` is a disjunction of equalities, whose
        // first disjunct evaluates the guards in their order: no wrapper
        (
            r#"[principal.name, "x"].contains(resource.owner.name)"#,
            r#"principal.name == resource.owner.name || "x" == resource.owner.name"#,
        ),
        // `containsAll` / `containsAny` with a literal right operand; a
        // literal left operand makes the `contains` disjunctions again
        (
            "[1, 2].containsAll([principal.age])",
            "1 == principal.age || 2 == principal.age",
        ),
        (
            "resource.level == 1 && [principal.age].containsAny([resource.level, 7])",
            "resource.level == 1 && (principal.age == resource.level || principal.age == 7)",
        ),
        // `isEmpty` on a set literal is decided, its elements guarded
        (
            "[principal.age].isEmpty()",
            "if principal.age == principal.age then false else false",
        ),
        // `in` over a set literal is a disjunction; literals and variables need no guard
        (
            r#"principal in [Group::"g1", Group::"g2"]"#,
            r#"principal in Group::"g1" || principal in Group::"g2""#,
        ),
        // a rewritten node under an erring node: the node's own error must
        // still precede its right sibling's — it becomes a guard, which the
        // term then evaluates first (dropped); the field's guard stays, as
        // the term evaluates it inside the `+`, not as a unit
        (
            "({x: principal.age}.x + 1) == resource.level + 1",
            "if principal.age == principal.age
             then principal.age + 1 == resource.level + 1 else false",
        ),
        // a left conjunct that evaluated establishes its strict subterms
        // (`resource.level` above); likewise the first split's wrapper
        // establishes `principal.age` for the elimination inside it
        (
            "[principal.age].contains(if c then 1 else 2)",
            "if principal.age == principal.age
             then (if c then principal.age == 1 else principal.age == 2) else false",
        ),
        // the context of a false `||`-left / `if`-test establishes the guard
        (
            "principal.age + 1 == 19 || [principal.age + 1].isEmpty()",
            "principal.age + 1 == 19 || false",
        ),
        (
            "if principal.age + 1 == 19 then false else [principal.age + 1].isEmpty()",
            "if principal.age + 1 == 19 then false else false",
        ),
        // the left-literal `containsAny` arm alone: the `contains` disjuncts
        // can err, so the literal's elements and the set keep their guards
        (
            "principal has tags && [principal.age, 7].containsAny(principal.tags)",
            "principal has tags
             && (if principal.age == principal.age && principal.tags == principal.tags
                 then principal.tags.contains(principal.age) || principal.tags.contains(7)
                 else false)",
        ),
        // `in` with an element that needs a guard
        (
            r#"principal has parent && principal in [Group::"g1", principal.parent]"#,
            r#"principal has parent
             && (if principal.parent == principal.parent
                 then principal in Group::"g1" || principal in principal.parent else false)"#,
        ),
        // a leaf erring twice in one atom is guarded once — and, deduped
        // before the term's own evaluation order is compared, not at all
        (
            "{x: principal.age, y: principal.age}.y == 1",
            "principal.age == 1",
        ),
        // set equality with a literal side: `containsAll` both ways, the
        // literal-right one an element-wise `contains` chain
        (
            "principal has tags && principal.tags == [1, 2]",
            "principal has tags
             && (if principal.tags == principal.tags
                 then (principal.tags.contains(1) && principal.tags.contains(2))
                      && [1, 2].containsAll(principal.tags)
                 else false)",
        ),
        // both sides literal: two chains of element equalities
        (
            "[principal.age, 7] == [7, principal.age]",
            "((principal.age == 7 || true) && (principal.age == principal.age || 7 == principal.age))
             && ((7 == principal.age || principal.age == principal.age) && (true || principal.age == 7))",
        ),
        // nested: a record field equality and a `contains` element equality
        (
            "principal has tags && {x: principal.tags, y: 1} == {x: [3], y: 1}",
            "principal has tags
             && (if principal.tags == principal.tags
                 then (principal.tags.contains(3) && [3].containsAll(principal.tags)) && true
                 else false)",
        ),
        (
            "principal has tags && [[principal.age]].contains(principal.tags)",
            "principal has tags
             && (if principal.age == principal.age && principal.tags == principal.tags
                 then ([principal.age].containsAll(principal.tags)
                       && principal.tags.contains(principal.age))
                 else false)",
        ),
        // clean of literals: unchanged
        ("a && principal.age == 1", "a && principal.age == 1"),
        // an `iferror` call is opaque
        (
            "iferror({x: principal.age}.x == 1, false)",
            "iferror({x: principal.age}.x == 1, false)",
        ),
    ]
}

#[test]
fn elim_tables() {
    let schema = schema();
    for (input, expected) in elim_table() {
        let input = normalize(input);
        let normalized =
            normalize_atoms(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES).unwrap();
        assert_eq!(
            normalized,
            normalize(expected),
            "expected `{expected}` for `{input}`, got `{normalized}`"
        );
        assert!(
            eliminated(&normalized),
            "`{normalized}` still has literals to eliminate"
        );
        assert!(
            atoms_are_clean(&normalized),
            "`{normalized}` has unclean atoms"
        );
        // on an already clean input the pass alone, re-split, agrees
        if atoms_are_clean(&input) {
            let alone =
                eliminate_aggregates(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES).unwrap();
            let resplit = split_atoms(&alone, DEFAULT_MAX_SPLIT_NODES).unwrap();
            assert_eq!(resplit, normalized, "for `{input}`");
        }
    }
}

#[tokio::test]
async fn elim_tables_are_solver_equivalent() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    for (input, _) in elim_table() {
        let input = normalize(input);
        let normalized =
            normalize_atoms(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES).unwrap();
        let annotated = with_default_metadata(&normalized).unwrap();
        assert!(
            ev.check_equivalent(&input, &annotated, &view(), no_extra())
                .await
                .unwrap(),
            "`{normalized}` is not solver-equivalent to `{input}`"
        );
        let dnf = Dnf::of_expr(&normalized).unwrap();
        assert_equivalent(&mut ev, &input, &dnf).await;
    }
}

/// The rewrites assume a well-typed input and refuse anything else.
/// The `contains` rule duplicates its argument per element, so nested
/// literals grow the rewritten atom exponentially as a tree while the
/// elimination itself, on shared structure, stays small: the budget rejects
/// the output before anything walks it as a tree.
#[test]
fn elim_budget() {
    let schema = schema();
    let mut text = "principal.active".to_string();
    for _ in 0..8 {
        text = format!("[true, true, true, true, true, true, true, true].contains({text})");
    }
    let input = normalize(&text);
    let started = std::time::Instant::now();
    assert!(matches!(
        normalize_atoms(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES),
        Err(DnfError::TooLarge { .. })
    ));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the budget did not stop the blow-up in time"
    );
    // a small nesting is fine
    let small = normalize("[true, false].contains([true, false].contains(principal.active))");
    assert!(normalize_atoms(&small, &schema, &view(), DEFAULT_MAX_SPLIT_NODES).is_ok());
}

#[test]
fn elim_rejects_ill_typed_input() {
    let schema = schema();
    for text in [
        "principal.nick == \"x\"", // an optional attribute without a `has` guard
        "principal.age == \"x\"",  // incomparable types
        "[1].containsAll(principal.name)",
    ] {
        let input = expr(text);
        assert!(
            matches!(
                eliminate_aggregates(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES),
                Err(DnfError::NotWellTyped { .. })
            ),
            "`{text}` should be rejected"
        );
        assert!(matches!(
            normalize_atoms(&input, &schema, &view(), DEFAULT_MAX_SPLIT_NODES),
            Err(DnfError::NotWellTyped { .. })
        ));
    }
}
/// `iferror` in the DNF pipeline: an `iferror` call is an atom (its root is
/// not `&&`/`||`/`!`/`if`) that the splitter never looks inside, structure
/// *around* it is hoisted as usual, and the results are solver-equivalent
/// to the originals.
/// Also the two laws part 2 of Step 4 rests on: `iferror(e, false)` never
/// errors, and it agrees with `e` wherever `e` does not error.
#[tokio::test]
async fn iferror_atoms() {
    let schema = schema();
    let mut ev = evaluator(&schema);

    // an atom: the DNF keeps it opaque
    let e = t("iferror(e, false) || a");
    let dnf = Dnf::of_expr(&e).unwrap();
    assert_eq!(dnf.cubes().len(), 2);
    assert!(dnf.cubes().iter().all(|c| !c.is_never_true()));
    assert_equivalent(&mut ev, &e, &dnf).await;

    // an `iferror` call is opaque: the structure inside it is *not* hoisted
    // (that would move `e`'s error outside the coalescing scope —
    // `if (a && e) then … else …` errors where `iferror(a && e, false)` is
    // `false`), so the split is the identity and its atoms count as clean
    let split = split_atoms(&t("iferror(a && e, false)"), DEFAULT_MAX_SPLIT_NODES).unwrap();
    assert_eq!(split, t("iferror(a && e, false)"));
    assert!(atoms_are_clean(&split), "{split}");
    // ... while structure *around* an `iferror` call is hoisted as usual
    let split = split_atoms(&t("(a && b) == iferror(e, false)"), DEFAULT_MAX_SPLIT_NODES).unwrap();
    assert_eq!(
        split,
        t("if (a && b) then true == iferror(e, false) else false == iferror(e, false)")
    );
    assert!(atoms_are_clean(&split), "{split}");
    assert!(ev
        .check_equivalent(
            &t("(a && b) == iferror(e, false)"),
            &with_default_metadata(&split).unwrap(),
            &view(),
            no_extra()
        )
        .await
        .unwrap());

    // never errors: `iferror(e, false) || !iferror(e, false)` is `true`
    let always = with_default_metadata(&expr("true")).unwrap();
    assert!(ev
        .check_equivalent(
            &t("iferror(e, false) || !iferror(e, false)"),
            &always,
            &view(),
            no_extra()
        )
        .await
        .unwrap());

    // agrees with `e` where `e` does not error
    let plain = with_default_metadata(&t("a")).unwrap();
    assert!(ev
        .check_equivalent(&t("iferror(a, false)"), &plain, &view(), no_extra())
        .await
        .unwrap());
    // ... and differs from `e` in general: `iferror(e, false)` is not `e`
    let erroring = with_default_metadata(&t("e")).unwrap();
    assert!(!ev
        .check_equivalent(&t("iferror(e, false)"), &erroring, &view(), no_extra())
        .await
        .unwrap());
}
// ---------------------------------------------------------------------------
// Phase 4 Step 1: `like` without wildcards is `==`

use cedar_policy_core::ast::PatternElem;
use cedar_policy_symcc::dnf::{likes_have_wildcards, rewrite_like};

/// Hand cases: a wildcard-free pattern becomes an equality (the empty
/// pattern too), an escaped `\*` is a character, a wildcard is kept, and
/// the rewrite reaches inside every node kind.
#[test]
fn rewrite_like_hand_cases() {
    let cases = [
        (
            r#"principal.name like "alice""#,
            r#"principal.name == "alice""#,
        ),
        (r#"principal.name like """#, r#"principal.name == """#),
        (
            r#"principal.name like "a\*b""#,
            r#"principal.name == "a*b""#,
        ),
        (
            r#"principal.name like "a*b""#,
            r#"principal.name like "a*b""#,
        ),
        (r#"principal.name like "*""#, r#"principal.name like "*""#),
        (
            r#"if principal.name like "x" then {a: principal.name like "y"}.a else [principal.name like "z*"].contains(true)"#,
            r#"if principal.name == "x" then {a: principal.name == "y"}.a else [principal.name like "z*"].contains(true)"#,
        ),
        (
            r#"iferror(principal.name like "x", principal.name like "y") && !(principal.name like "w")"#,
            r#"iferror(principal.name == "x", principal.name == "y") && !(principal.name == "w")"#,
        ),
        (
            r#"(principal.name like "x") == (principal.name like "*x")"#,
            r#"(principal.name == "x") == (principal.name like "*x")"#,
        ),
    ];
    for (input, expected) in cases {
        let rewritten = rewrite_like(&expr(input)).unwrap();
        assert_eq!(rewritten, expr(expected), "{input}");
        assert!(likes_have_wildcards(&rewritten), "{input}");
    }
    assert!(!likes_have_wildcards(&expr(
        r#"principal.name like "x" || false"#
    )));
    assert!(likes_have_wildcards(&expr(
        r#"principal.name like "x*" || false"#
    )));
}

/// The rewrite is solver-equivalent to its input on well-typed
/// expressions, and a wildcard-free pattern matches exactly the literal it
/// becomes under the concrete matcher. (On a non-string *value* `like` is
/// a type error while `==` is `false`; the rewrite is meant for validated
/// expressions, whose `like` operands are strings.)
#[tokio::test]
async fn rewrite_like_is_equivalent() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    for text in [
        r#"principal.name like "alice""#,
        r#"principal.name like """#,
        r#"principal.name like "a\*b" && principal.active"#,
        r#"principal.name like "a*b" || principal.name like "c""#,
        r#"if principal.name like "x" then principal.age > 18 else principal.name like "*""#,
        r#"iferror(principal.name like "x", false)"#,
    ] {
        let input = expr(text);
        let rewritten = with_default_metadata(&rewrite_like(&input).unwrap()).unwrap();
        assert!(
            ev.check_equivalent(&input, &rewritten, &view(), no_extra())
                .await
                .unwrap(),
            "`{rewritten}` is not solver-equivalent to `{input}`"
        );
    }
    // concretely: the same verdict on every string, wildcard or not
    let pattern = |src: &str| match expr(src).expr_kind() {
        ExprKind::Like { pattern, .. } => pattern.clone(),
        _ => panic!("not a like"),
    };
    for (src, matches, misses) in [
        (
            r#"context.s like "ab""#,
            vec!["ab"],
            vec!["", "a", "abc", "ba"],
        ),
        (r#"context.s like "a\*b""#, vec!["a*b"], vec!["ab", "axb"]),
        (
            r#"context.s like "a*b""#,
            vec!["ab", "axb", "a**b"],
            vec!["a", "b", "ba"],
        ),
    ] {
        let p = pattern(src);
        let wildcard = p.iter().any(|e| matches!(e, PatternElem::Wildcard));
        for s in matches {
            assert!(p.wildcard_match(s), "{src} should match {s:?}");
        }
        for s in misses {
            assert!(!p.wildcard_match(s), "{src} should not match {s:?}");
        }
        let rewritten = rewrite_like(&expr(src)).unwrap();
        assert_eq!(
            matches!(rewritten.expr_kind(), ExprKind::Like { .. }),
            wildcard,
            "{src}"
        );
        // a wildcard-free pattern matches exactly the literal it becomes
        if let ExprKind::BinaryApp { arg2, .. } = rewritten.expr_kind() {
            let ExprKind::Lit(Literal::String(lit)) = arg2.expr_kind() else {
                panic!("{src}: expected a string literal, got {arg2}");
            };
            for s in ["", "a", "ab", "a*b", "axb", "abc", "ba"] {
                assert_eq!(p.wildcard_match(s), s == lit.as_str(), "{src} on {s:?}");
            }
        }
    }
}
