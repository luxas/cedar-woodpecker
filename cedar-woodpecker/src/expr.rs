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

//! Expression rewrites: renaming the request variables of a source cube to
//! the intermediate request of the target environment, and spotting the
//! conjuncts that mention it.

use std::sync::Arc;

use cedar_policy_core::ast::{BinaryOp, Expr, ExprBuilder, ExprKind, Literal, UnaryOp, Var};
use cedar_policy_core::expr_builder::ExprBuilder as _;
use smol_str::SmolStr;

/// `context.<attr>`.
pub fn context_attr(attr: &str) -> Expr {
    Expr::get_attr(Expr::var(Var::Context), attr.into())
}

/// The children of a node, in evaluation order.
pub fn children(e: &Expr) -> Vec<&Expr> {
    match e.expr_kind() {
        ExprKind::Lit(_) | ExprKind::Var(_) | ExprKind::Slot(_) | ExprKind::Unknown(_) => vec![],
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => vec![test_expr, then_expr, else_expr],
        ExprKind::And { left, right } | ExprKind::Or { left, right } => vec![left, right],
        ExprKind::UnaryApp { arg, .. } => vec![arg],
        ExprKind::BinaryApp { arg1, arg2, .. } => vec![arg1, arg2],
        ExprKind::ExtensionFunctionApp { args, .. } => args.iter().collect(),
        ExprKind::GetAttr { expr, .. }
        | ExprKind::HasAttr { expr, .. }
        | ExprKind::Like { expr, .. }
        | ExprKind::Is { expr, .. } => vec![expr],
        ExprKind::Set(xs) => xs.iter().collect(),
        ExprKind::Record(m) => m.values().collect(),
        #[expect(
            unreachable_patterns,
            reason = "only reachable with the `tolerant-ast` feature"
        )]
        _ => vec![],
    }
}

/// `e` with `f` applied to every child (the node itself rebuilt around the
/// results, structure and source location kept).
pub fn rebuild_children(e: &Expr, f: impl Fn(&Expr) -> Expr) -> Expr {
    let child = |c: &Arc<Expr>| Arc::new(f(c));
    let kind = match e.expr_kind() {
        ExprKind::Lit(_) | ExprKind::Var(_) | ExprKind::Slot(_) | ExprKind::Unknown(_) => {
            return e.clone();
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => ExprKind::If {
            test_expr: child(test_expr),
            then_expr: child(then_expr),
            else_expr: child(else_expr),
        },
        ExprKind::And { left, right } => ExprKind::And {
            left: child(left),
            right: child(right),
        },
        ExprKind::Or { left, right } => ExprKind::Or {
            left: child(left),
            right: child(right),
        },
        ExprKind::UnaryApp { op, arg } => ExprKind::UnaryApp {
            op: *op,
            arg: child(arg),
        },
        ExprKind::BinaryApp { op, arg1, arg2 } => ExprKind::BinaryApp {
            op: *op,
            arg1: child(arg1),
            arg2: child(arg2),
        },
        ExprKind::ExtensionFunctionApp { fn_name, args } => ExprKind::ExtensionFunctionApp {
            fn_name: fn_name.clone(),
            args: Arc::new(args.iter().map(&f).collect()),
        },
        ExprKind::GetAttr { expr, attr } => ExprKind::GetAttr {
            expr: child(expr),
            attr: attr.clone(),
        },
        ExprKind::HasAttr { expr, attr } => ExprKind::HasAttr {
            expr: child(expr),
            attr: attr.clone(),
        },
        ExprKind::Like { expr, pattern } => ExprKind::Like {
            expr: child(expr),
            pattern: pattern.clone(),
        },
        ExprKind::Is { expr, entity_type } => ExprKind::Is {
            expr: child(expr),
            entity_type: entity_type.clone(),
        },
        ExprKind::Set(xs) => ExprKind::Set(Arc::new(xs.iter().map(&f).collect())),
        ExprKind::Record(m) => {
            ExprKind::Record(Arc::new(m.iter().map(|(k, v)| (k.clone(), f(v))).collect()))
        }
        #[expect(
            unreachable_patterns,
            reason = "only reachable with the `tolerant-ast` feature"
        )]
        _ => return e.clone(),
    };
    ExprBuilder::with_data(())
        .with_same_source_loc(e)
        .with_expr_kind(kind)
}

/// Rewrites `e` top-down: where `f` gives a replacement the node is replaced
/// (and not descended into), elsewhere the children are rewritten.
pub fn map_expr(e: &Expr, f: &impl Fn(&Expr) -> Option<Expr>) -> Expr {
    f(e).unwrap_or_else(|| rebuild_children(e, |c| map_expr(c, f)))
}

/// `e` with every occurrence of `from` (structurally, locations ignored)
/// replaced by `to`.
pub fn replace(e: &Expr, from: &Expr, to: &Expr) -> Expr {
    map_expr(e, &|n| (n == from).then(|| to.clone()))
}

/// Whether `e` occurs in `haystack` (structurally, locations ignored).
pub fn occurs(e: &Expr, haystack: &Expr) -> bool {
    haystack.subexpressions().any(|s| s == e)
}

/// Whether `e` is a constant: a literal, or a set, record or extension
/// constructor call over constants.
pub fn is_constant(e: &Expr) -> bool {
    match e.expr_kind() {
        ExprKind::Lit(_) => true,
        ExprKind::Set(_) | ExprKind::Record(_) | ExprKind::ExtensionFunctionApp { .. } => {
            children(e).into_iter().all(is_constant)
        }
        _ => false,
    }
}

/// Local folds, bottom-up: a record literal under `.a` / `has a`, an
/// equality of two primitive literals, and `!` of a boolean literal.
pub fn simplify(e: &Expr) -> Expr {
    let e = rebuild_children(e, simplify);
    match e.expr_kind() {
        ExprKind::GetAttr { expr, attr } => match expr.expr_kind() {
            ExprKind::Record(m) => m.get(attr).cloned().unwrap_or(e),
            _ => e,
        },
        ExprKind::HasAttr { expr, attr } => match expr.expr_kind() {
            ExprKind::Record(m) => Expr::val(m.contains_key(attr)),
            _ => e,
        },
        ExprKind::BinaryApp {
            op: BinaryOp::Eq,
            arg1,
            arg2,
        } => match (arg1.expr_kind(), arg2.expr_kind()) {
            (ExprKind::Lit(l1), ExprKind::Lit(l2)) => Expr::val(l1 == l2),
            _ => e,
        },
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => match arg.expr_kind() {
            ExprKind::Lit(Literal::Bool(b)) => Expr::val(!b),
            _ => e,
        },
        _ => e,
    }
}

/// `expr` with every `action` variable replaced by `action`, every
/// `resource` by `resource` and every `context` by `context` (the source
/// cube's variables becoming the intermediate request of the target
/// environment), structure and source locations kept.
pub fn rename_vars(expr: &Expr, action: &Expr, resource: &Expr, context: &Expr) -> Expr {
    map_expr(expr, &|n| match n.expr_kind() {
        ExprKind::Var(Var::Action) => Some(action.clone()),
        ExprKind::Var(Var::Resource) => Some(resource.clone()),
        ExprKind::Var(Var::Context) => Some(context.clone()),
        _ => None,
    })
}

/// Whether `expr` mentions any of the context attributes `attrs`: a
/// `context.a` / `context has a` with `a` in `attrs`, or `context` used as
/// a whole (which may denote them).
pub fn mentions_context_attrs(expr: &Expr, attrs: &[SmolStr]) -> bool {
    match expr.expr_kind() {
        ExprKind::GetAttr { expr: e, attr } | ExprKind::HasAttr { expr: e, attr }
            if matches!(e.expr_kind(), ExprKind::Var(Var::Context)) =>
        {
            attrs.contains(attr)
        }
        ExprKind::Var(Var::Context) => true,
        _ => children(expr)
            .into_iter()
            .any(|c| mentions_context_attrs(c, attrs)),
    }
}

/// The right-nested `&&` chain of `conjuncts`; `None` when there are none.
pub fn and_chain(conjuncts: impl IntoIterator<Item = Expr>) -> Option<Expr> {
    let mut items: Vec<Expr> = conjuncts.into_iter().collect();
    let last = items.pop()?;
    Some(items.into_iter().rev().fold(last, |acc, e| {
        ExprBuilder::with_data(()).with_expr_kind(ExprKind::And {
            left: Arc::new(e),
            right: Arc::new(acc),
        })
    }))
}

/// The right-nested `||` chain of `disjuncts`; `None` when there are none.
pub fn or_chain(disjuncts: impl IntoIterator<Item = Expr>) -> Option<Expr> {
    let mut items: Vec<Expr> = disjuncts.into_iter().collect();
    let last = items.pop()?;
    Some(items.into_iter().rev().fold(last, |acc, e| {
        ExprBuilder::with_data(()).with_expr_kind(ExprKind::Or {
            left: Arc::new(e),
            right: Arc::new(acc),
        })
    }))
}
