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

//! Structure-preserving rebuilds of [`Expr`] trees, shared by the modules
//! that carry their own per-node data (the symbolic evaluator, the DNF
//! converter). The one exhaustive `ExprKind` traversal lives here
//! ([`children`] / [`with_children`]); everything else is built on top.

use std::sync::Arc;

use cedar_policy_core::ast::{Expr, ExprBuilder, ExprKind};
use cedar_policy_core::evaluator::stack_size_check;
use cedar_policy_core::expr_builder::ExprBuilder as _;

/// Errors of the rebuild helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebuildError {
    /// An expression kind the rebuild does not handle (only the
    /// `tolerant-ast` error node).
    Unsupported(&'static str),
    /// The expression nests too deeply (same limit as the concrete
    /// evaluator).
    RecursionLimit,
}

/// The direct children of an expression, in evaluation order.
pub(crate) fn children<T>(e: &Expr<T>) -> Vec<&Expr<T>> {
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
        ExprKind::Set(elems) => elems.iter().collect(),
        ExprKind::Record(map) => map.values().collect(),
        #[expect(
            unreachable_patterns,
            reason = "only reachable with the `tolerant-ast` feature"
        )]
        _ => vec![],
    }
}

/// Rebuilds `e` with the given children — which must be as many as
/// [`children`] returns, in the same order — and `data` at the root,
/// preserving the node's non-child parts and source location.
pub(crate) fn with_children<T, U: Clone + Default>(
    e: &Expr<T>,
    new: Vec<Expr<U>>,
    data: U,
) -> Result<Expr<U>, RebuildError> {
    let mut new = new.into_iter().map(Arc::new);
    let mut next = || {
        new.next()
            .ok_or(RebuildError::Unsupported("missing child in rebuild"))
    };
    let kind = match e.expr_kind() {
        ExprKind::Lit(l) => ExprKind::Lit(l.clone()),
        ExprKind::Var(v) => ExprKind::Var(*v),
        ExprKind::Slot(s) => ExprKind::Slot(*s),
        ExprKind::Unknown(u) => ExprKind::Unknown(u.clone()),
        ExprKind::If { .. } => ExprKind::If {
            test_expr: next()?,
            then_expr: next()?,
            else_expr: next()?,
        },
        ExprKind::And { .. } => ExprKind::And {
            left: next()?,
            right: next()?,
        },
        ExprKind::Or { .. } => ExprKind::Or {
            left: next()?,
            right: next()?,
        },
        ExprKind::UnaryApp { op, .. } => ExprKind::UnaryApp {
            op: *op,
            arg: next()?,
        },
        ExprKind::BinaryApp { op, .. } => ExprKind::BinaryApp {
            op: *op,
            arg1: next()?,
            arg2: next()?,
        },
        ExprKind::ExtensionFunctionApp { fn_name, args } => ExprKind::ExtensionFunctionApp {
            fn_name: fn_name.clone(),
            args: Arc::new(
                (0..args.len())
                    .map(|_| next().map(|a| (*a).clone()))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        },
        ExprKind::GetAttr { attr, .. } => ExprKind::GetAttr {
            expr: next()?,
            attr: attr.clone(),
        },
        ExprKind::HasAttr { attr, .. } => ExprKind::HasAttr {
            expr: next()?,
            attr: attr.clone(),
        },
        ExprKind::Like { pattern, .. } => ExprKind::Like {
            expr: next()?,
            pattern: pattern.clone(),
        },
        ExprKind::Is { entity_type, .. } => ExprKind::Is {
            expr: next()?,
            entity_type: entity_type.clone(),
        },
        ExprKind::Set(elems) => ExprKind::Set(Arc::new(
            (0..elems.len())
                .map(|_| next().map(|a| (*a).clone()))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ExprKind::Record(map) => ExprKind::Record(Arc::new(
            map.keys()
                .map(|k| next().map(|v| (k.clone(), (*v).clone())))
                .collect::<Result<_, _>>()?,
        )),
        #[expect(
            unreachable_patterns,
            reason = "only reachable with the `tolerant-ast` feature"
        )]
        _ => {
            return Err(RebuildError::Unsupported(
                "unsupported expression kind (tolerant-ast error node)",
            ))
        }
    };
    if new.next().is_some() {
        return Err(RebuildError::Unsupported("extra child in rebuild"));
    }
    Ok(ExprBuilder::with_data(data)
        .with_same_source_loc(e)
        .with_expr_kind(kind))
}

/// Rebuilds `expr` with `data` at the root and `U::default()` at every other
/// node, preserving structure and source locations exactly.
///
/// `Expr::into_expr` is deliberately not used: it goes through
/// `ExprBuilder::and`/`or`, which constant-fold literal operands and would
/// change the structure.
pub(crate) fn with_data<T: Clone, U: Clone + Default>(
    expr: &Expr<T>,
    data: U,
) -> Result<Expr<U>, RebuildError> {
    stack_size_check().map_err(|_| RebuildError::RecursionLimit)?;
    let new = children(expr)
        .into_iter()
        .map(|c| with_data(c, U::default()))
        .collect::<Result<Vec<_>, _>>()?;
    with_children(expr, new, data)
}

/// Rebuilds `expr` with `f` applied to every node's data, preserving
/// structure and source locations exactly.
pub(crate) fn map_data<T: Clone, U: Clone + Default>(
    expr: &Expr<T>,
    f: &impl Fn(&T) -> U,
) -> Result<Expr<U>, RebuildError> {
    stack_size_check().map_err(|_| RebuildError::RecursionLimit)?;
    let new = children(expr)
        .into_iter()
        .map(|c| map_data(c, f))
        .collect::<Result<Vec<_>, _>>()?;
    with_children(expr, new, f(expr.data()))
}

/// Strips the data from an expression.
pub(crate) fn erase<T: Clone>(expr: &Expr<T>) -> Result<Expr<()>, RebuildError> {
    with_data(expr, ())
}
