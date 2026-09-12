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

//! Cedar's three-valued short-circuit semantics over opaque atoms.

use cedar_policy_core::ast::{Expr, ExprKind, Literal, UnaryOp};
use cedar_policy_core::tpe::residual::EvaluationOutcome;

/// The outcome of the boolean structure of `expr` when each atom takes the
/// outcome `atoms` assigns to it: `&&`, `||`, `!` and `if` follow Cedar's
/// left-to-right, short-circuiting, error-propagating semantics, `iferror`
/// coalesces an error into its fallback, and every other node is passed to
/// `atoms`.
///
/// This is the truth-table oracle for the DNF conversion: an expression and
/// its DNF must agree for every assignment of the atoms.
pub fn interpret<T>(
    expr: &Expr<T>,
    mut atoms: impl FnMut(&Expr<T>) -> EvaluationOutcome,
) -> EvaluationOutcome {
    go(expr, &mut atoms)
}

fn go<T>(
    expr: &Expr<T>,
    atoms: &mut impl FnMut(&Expr<T>) -> EvaluationOutcome,
) -> EvaluationOutcome {
    use EvaluationOutcome::{Error, False, True};
    match expr.expr_kind() {
        ExprKind::Lit(Literal::Bool(true)) => True,
        ExprKind::Lit(Literal::Bool(false)) => False,
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => match go(arg, atoms) {
            True => False,
            False => True,
            Error => Error,
        },
        ExprKind::And { left, right } => match go(left, atoms) {
            True => go(right, atoms),
            other => other,
        },
        ExprKind::Or { left, right } => match go(left, atoms) {
            False => go(right, atoms),
            other => other,
        },
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => match go(test_expr, atoms) {
            True => go(then_expr, atoms),
            False => go(else_expr, atoms),
            Error => Error,
        },
        // `iferror(e, d)` coalesces `e`'s error into `d`'s boolean, with `d`
        // evaluated only then (Step 4); a boolean `e` passes through
        ExprKind::ExtensionFunctionApp { fn_name, args }
            if cedar_policy_core::extensions::iferror::is_iferror(fn_name) =>
        {
            match args.as_slice() {
                [e, d] => match go(e, atoms) {
                    Error => go(d, atoms),
                    other => other,
                },
                _ => atoms(expr),
            }
        }
        _ => atoms(expr),
    }
}
