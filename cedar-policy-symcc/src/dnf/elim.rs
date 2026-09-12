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

//! Eliminating record and set literals from atoms (Phase 4 Step 1): the
//! structure a literal hides under `.attr` / `has` / `==` / `contains` /
//! `containsAll` / `containsAny` / `isEmpty` / `in` is rewritten into the
//! operations on its elements. See [`eliminate_aggregates`].

use std::collections::HashMap;
use std::sync::Arc;

use cedar_policy::{RequestEnv, Schema};
use cedar_policy_core::ast::{
    BinaryOp, Expr, ExprBuilder, ExprKind, Literal as AstLiteral, UnaryOp,
};
use cedar_policy_core::evaluator::stack_size_check;
use cedar_policy_core::expr_builder::ExprBuilder as _;

use super::split::{
    add_guards, erase_and_fold, extend, guard, is_iferror_call, is_offender, learn_false,
    learn_true, never_errs_itself, new_guards, rebuild_err, split_atoms, Budget,
};
use super::{and, and_chain, bool_lit, or_chain, DnfError};
use crate::evaluator::EvaluationError;
use crate::expr_util::{children, erase, with_children};

/// Rewrites the boolean expression `expr` so that no atom contains a record
/// literal under `.attr` or `has`, two under `==`, a set literal as the
/// left operand of `contains` or `containsAny`, the right operand of
/// `containsAll`, `containsAny` or `in`, either side of `==`, or under
/// `isEmpty`, preserving Cedar's evaluation semantics exactly — the same
/// value, or the same error — on every input on which `expr` is well typed.
/// A literal in any other position survives: `<set>.contains(<record>)`, a
/// record literal on one side of `==` only, and a set literal on the left of
/// `containsAll` (see the README). The rules, applied bottom-up (children
/// first, so their results are already rewritten):
///
/// ```text
/// {a: x, …}.a            ⇒ x                       (the first field named a)
/// {a: x, …} has a        ⇒ true / false
/// {a: x, …} == {a: y, …} ⇒ x == y && …              (same key sets)
/// a == b, a set literal on either side
///                         ⇒ a.containsAll(b) && b.containsAll(a)
/// [l₁, …, lₙ].contains(x) ⇒ l₁ == x || … || lₙ == x
/// s.containsAll([l₁, …])  ⇒ s.contains(l₁) && …
/// s.containsAny([l₁, …])  ⇒ s.contains(l₁) || …      (and symmetrically)
/// [l₁, …].isEmpty()       ⇒ false / true
/// e in [x₁, …, xₙ]        ⇒ e in x₁ || … || e in xₙ
/// ```
///
/// where an equality, `contains` or `containsAll` a rule produces is itself
/// rewritten when it meets literals again (`{a: {b: 1}} == {a: {b: 2}}` ends
/// as `1 == 2`; `s == [l]` as `s.contains(l) && [l].containsAll(s)`), and the
/// empty chains are `true` (`&&`) and `false` (`||`).
///
/// **Typing.** The rules assume the expression is well typed against
/// `schema` in `req_env`, and this function checks that first, failing with
/// [`DnfError::NotWellTyped`] otherwise. The assumption is what makes the
/// rules over a set-typed operand exact: `in` over a set type-checks *every*
/// element before answering while the disjunction stops at the first match,
/// `s.containsAll([])` is `true` only for a set-typed `s`, and `s == [l…]`
/// is `false`, not an error, for a non-set `s` where the `containsAll` pair
/// type-errors. Typing rules every corner out.
///
/// **Errors.** A rewrite reorders and drops evaluation: `{a: x, b: y}.b`
/// never evaluates `x` once rewritten to `y`. Like [`split_atoms`], the pass
/// keeps the original error behaviour with guards: every atom that was
/// rewritten becomes `if (g₁ == g₁ && …) then atom' else false`, where the
/// guards are, in evaluation order, the subterms the original evaluated —
/// every maximal unrewritten subterm (a *leaf region*, decomposed like a
/// split guard) and, for an unrewritten node above a rewritten one, the node
/// itself after rewriting (its own error, say an overflow, must still
/// precede its right siblings'); rewritten nodes never err once their
/// children have evaluated, so they need no guard of their own. Guards the
/// rewritten atom evaluates itself, first and in the same order, are dropped
/// again ([`drop_repeated`]): `[1, 2].contains(p.age)` becomes plainly
/// `1 == p.age || 2 == p.age`. An `iferror(…)` call, and any `&&`/`||`/`!`/`if`
/// inside an atom, is an opaque leaf region: the pass is exact on any input,
/// but *complete* — every literal in a position above reached — only on
/// split input, where no such node hides a literal (`(if c then {a: 1}
/// else {a: 2}).a` is left alone until the `if` is hoisted). So the pass
/// belongs between two runs of [`split_atoms`] (see [`normalize_atoms`]):
/// the first exposes every literal, the second hoists the `&&`/`||` the
/// rules introduce.
///
/// The `contains`, `containsAll`, `containsAny` and `in` rules duplicate one
/// operand per element, so the result can be exponentially larger than the
/// input as a tree (the copies share structure); `max_nodes` bounds the tree
/// size of the rewritten atoms, and exhausting it fails with
/// [`DnfError::TooLarge`].
pub fn eliminate_aggregates(
    expr: &Expr,
    schema: &Schema,
    req_env: &RequestEnv,
    max_nodes: usize,
) -> Result<Expr, DnfError> {
    typecheck(expr, schema, req_env)?;
    Ok(elim_structure(expr, &[], &mut Budget::new(max_nodes))?.0)
}

/// The pipeline `split_atoms` → [`eliminate_aggregates`] → `split_atoms`:
/// every atom of the result is clean (no `&&`/`||`/`!`/`if` outside an
/// `iferror` call) and free of the literals the rules eliminate, and the
/// result evaluates exactly like `expr` on every input. `expr` must be well
/// typed against `schema` in `req_env` ([`DnfError::NotWellTyped`]);
/// `max_nodes` is each pass's budget.
pub fn normalize_atoms(
    expr: &Expr,
    schema: &Schema,
    req_env: &RequestEnv,
    max_nodes: usize,
) -> Result<Expr, DnfError> {
    typecheck(expr, schema, req_env)?;
    normalize_unchecked(expr, max_nodes)
}

/// [`normalize_atoms`] for an input the caller has already checked to be
/// well typed (a validated policy's condition).
pub(super) fn normalize_unchecked<T: Clone>(
    expr: &Expr<T>,
    max_nodes: usize,
) -> Result<Expr, DnfError> {
    let split = split_atoms(expr, max_nodes)?;
    let (eliminated, changed) = elim_structure(&split, &[], &mut Budget::new(max_nodes))?;
    if !changed {
        // Nothing to hoist: the second split would return its input.
        return Ok(split);
    }
    split_atoms(&eliminated, max_nodes)
}

/// Typechecks `expr` in `req_env` the way the symbolic evaluator does
/// (`evaluator::typecheck`: core exposes no expression typechecker, so
/// `expr` is wrapped in a `permit` policy whose condition is typechecked).
fn typecheck(expr: &Expr, schema: &Schema, req_env: &RequestEnv) -> Result<(), DnfError> {
    crate::evaluator::typecheck_expr(expr, req_env, schema)
        .map(|_| ())
        .map_err(|e| match e {
            EvaluationError::NotWellTyped { errs } => DnfError::NotWellTyped { errs },
            EvaluationError::RequestEnvNotFound(env) => DnfError::RequestEnvNotFound(env),
            other => DnfError::Typecheck(other.to_string()),
        })
}

/// Recurses through the `&&`/`||`/`!`/`if` structure and eliminates in every
/// atom; reports whether any atom was rewritten. `ctx` is the splitter's
/// context (`split_structure`): the subterms known to have evaluated here —
/// the enclosing wrappers' guards and the enclosing conditions' strict
/// subterms — which a rewritten atom's guards need not repeat.
fn elim_structure<T: Clone>(
    expr: &Expr<T>,
    ctx: &[Expr<()>],
    budget: &mut Budget,
) -> Result<(Expr, bool), DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    let node = |kind| ExprBuilder::new().with_expr_kind(kind);
    Ok(match expr.expr_kind() {
        ExprKind::Lit(AstLiteral::Bool(b)) => (bool_lit(*b), false),
        ExprKind::And { left, right } => {
            let inner = extend(ctx, learn_true(left)?);
            let (l, cl) = elim_structure(left, ctx, budget)?;
            let (r, cr) = elim_structure(right, &inner, budget)?;
            (
                node(ExprKind::And {
                    left: Arc::new(l),
                    right: Arc::new(r),
                }),
                cl || cr,
            )
        }
        ExprKind::Or { left, right } => {
            let inner = extend(ctx, learn_false(left)?);
            let (l, cl) = elim_structure(left, ctx, budget)?;
            let (r, cr) = elim_structure(right, &inner, budget)?;
            (
                node(ExprKind::Or {
                    left: Arc::new(l),
                    right: Arc::new(r),
                }),
                cl || cr,
            )
        }
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => {
            let (a, ca) = elim_structure(arg, ctx, budget)?;
            (
                node(ExprKind::UnaryApp {
                    op: UnaryOp::Not,
                    arg: Arc::new(a),
                }),
                ca,
            )
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => {
            let inner_then = extend(ctx, learn_true(test_expr)?);
            let inner_else = extend(ctx, learn_false(test_expr)?);
            let (c, cc) = elim_structure(test_expr, ctx, budget)?;
            let (t, ct) = elim_structure(then_expr, &inner_then, budget)?;
            let (e, ce) = elim_structure(else_expr, &inner_else, budget)?;
            (
                node(ExprKind::If {
                    test_expr: Arc::new(c),
                    then_expr: Arc::new(t),
                    else_expr: Arc::new(e),
                }),
                cc || ct || ce,
            )
        }
        _ => match elim(expr)? {
            Elim::Same(term) => (term, false),
            Elim::Changed { guards, term } => {
                // dedupe first: a guard listed twice would otherwise hide
                // the term's own evaluation of it
                let guards = drop_repeated(new_guards(ctx, guards), &term);
                let wrapped = guard(guards, term);
                budget.charge(tree_size(&wrapped))?;
                (wrapped, true)
            }
        },
    })
}

/// The tree size of `e` — the node count `split_atoms` will charge for it —
/// computed on the DAG: the rules share the operand they duplicate, so a
/// subterm is counted once per distinct node and the result costs the shared
/// size, not the tree's.
fn tree_size(e: &Expr<()>) -> usize {
    fn go(e: &Expr<()>, memo: &mut HashMap<*const Expr<()>, usize>) -> usize {
        let key = e as *const Expr<()>;
        if let Some(&n) = memo.get(&key) {
            return n;
        }
        let n = 1 + children(e)
            .into_iter()
            .map(|child| go(child, memo))
            .fold(0usize, usize::saturating_add);
        memo.insert(key, n);
        n
    }
    go(e, &mut HashMap::new())
}

/// The result of [`elim`]: the erased node when nothing inside was rewritten
/// (a leaf region), else the guards — the subterms the original evaluates
/// before the rewritten term would, in evaluation order — and the rewritten
/// term.
enum Elim {
    Same(Expr<()>),
    Changed {
        guards: Vec<Expr<()>>,
        term: Expr<()>,
    },
}

/// Eliminates inside `e`; see [`Elim`].
fn elim<T: Clone>(e: &Expr<T>) -> Result<Elim, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    if is_iferror_call(e) || is_offender(e) {
        return Ok(Elim::Same(erase(e).map_err(rebuild_err)?));
    }
    let kids = children(e)
        .into_iter()
        .map(elim)
        .collect::<Result<Vec<_>, DnfError>>()?;
    let any_changed = kids.iter().any(|k| matches!(k, Elim::Changed { .. }));
    // Without a rewrite below, only a rule at this node can change anything;
    // the children are then leaf regions.
    let mut guards = Vec::new();
    let mut terms = Vec::with_capacity(kids.len());
    for kid in kids {
        match kid {
            Elim::Changed { guards: g, term } => {
                guards.extend(g);
                terms.push(term);
            }
            Elim::Same(term) => {
                add_guards(erase_and_fold(&term)?, &mut guards);
                terms.push(term);
            }
        }
    }
    let rebuilt = with_children(e, terms, ()).map_err(rebuild_err)?;
    if let Some(term) = rewrite(&rebuilt) {
        return Ok(Elim::Changed { guards, term });
    }
    if !any_changed {
        return Ok(Elim::Same(rebuilt));
    }
    // An unrewritten node above a rewritten one: its own error must precede
    // its right siblings' — unless it cannot err once its children have.
    if !never_errs_itself(&rebuilt) {
        guards.push(rebuilt.clone());
    }
    Ok(Elim::Changed {
        guards,
        term: rebuilt,
    })
}

/// The rule for a node whose children are already rewritten, if one applies.
fn rewrite(e: &Expr<()>) -> Option<Expr<()>> {
    match e.expr_kind() {
        ExprKind::GetAttr { expr, attr } => match expr.expr_kind() {
            ExprKind::Record(map) => map.get(attr).cloned(),
            _ => None,
        },
        ExprKind::HasAttr { expr, attr } => match expr.expr_kind() {
            ExprKind::Record(map) => Some(bool_lit(map.contains_key(attr))),
            _ => None,
        },
        ExprKind::UnaryApp {
            op: UnaryOp::IsEmpty,
            arg,
        } => match arg.expr_kind() {
            ExprKind::Set(elems) => Some(bool_lit(elems.is_empty())),
            _ => None,
        },
        ExprKind::BinaryApp { op, arg1, arg2 } => match (op, arg1.expr_kind(), arg2.expr_kind()) {
            (BinaryOp::Eq, ExprKind::Record(_), ExprKind::Record(_)) => eq_records(arg1, arg2),
            (BinaryOp::Eq, ExprKind::Set(_), _) | (BinaryOp::Eq, _, ExprKind::Set(_)) => {
                Some(set_eq(arg1, arg2))
            }
            (BinaryOp::Contains, ExprKind::Set(elems), _) => Some(contains_literal(elems, arg2)),
            (BinaryOp::ContainsAll, _, ExprKind::Set(_)) => Some(contains_all(arg1, arg2)),
            (BinaryOp::ContainsAny, _, ExprKind::Set(elems)) => {
                Some(or_chain(elems.iter().map(|l| contains(arg1, l.clone()))))
            }
            (BinaryOp::ContainsAny, ExprKind::Set(elems), _) => {
                Some(or_chain(elems.iter().map(|l| contains(arg2, l.clone()))))
            }
            (BinaryOp::In, _, ExprKind::Set(elems)) => Some(or_chain(
                elems
                    .iter()
                    .map(|x| Expr::is_in(arg1.as_ref().clone(), x.clone())),
            )),
            _ => None,
        },
        _ => None,
    }
}

/// `{…} == {…}` with the same key set: the conjunction of the per-key value
/// equalities (each rewritten again where it meets literals); `None` for
/// different key sets, which do not typecheck (the plain equality is exact
/// there too).
fn eq_records(r1: &Expr<()>, r2: &Expr<()>) -> Option<Expr<()>> {
    let (ExprKind::Record(m1), ExprKind::Record(m2)) = (r1.expr_kind(), r2.expr_kind()) else {
        return None;
    };
    if !m1.keys().eq(m2.keys()) {
        return None;
    }
    Some(and_chain(m1.iter().map(|(k, v1)| eq(v1, &m2[k]))))
}

/// `a == b`, rewritten when both are record literals or one is a set
/// literal.
fn eq(a: &Expr<()>, b: &Expr<()>) -> Expr<()> {
    match (a.expr_kind(), b.expr_kind()) {
        (ExprKind::Record(_), ExprKind::Record(_)) => eq_records(a, b),
        (ExprKind::Set(_), _) | (_, ExprKind::Set(_)) => Some(set_eq(a, b)),
        _ => None,
    }
    .unwrap_or_else(|| Expr::is_eq(a.clone(), b.clone()))
}

/// `a == b` with a set literal on either side: `a.containsAll(b) &&
/// b.containsAll(a)`, each rewritten where its right operand is the literal.
/// Exact for two set values, which typing gives (and, in the Lean model,
/// well-formed ones: `Set.subset_iff_eq`).
fn set_eq(a: &Expr<()>, b: &Expr<()>) -> Expr<()> {
    and(contains_all(a, b), contains_all(b, a))
}

/// `s.containsAll(t)`, rewritten when `t` is a set literal.
fn contains_all(s: &Expr<()>, t: &Expr<()>) -> Expr<()> {
    match t.expr_kind() {
        ExprKind::Set(elems) => and_chain(elems.iter().map(|l| contains(s, l.clone()))),
        _ => Expr::contains_all(s.clone(), t.clone()),
    }
}

/// `s.contains(x)`, rewritten when `s` is a set literal.
fn contains(s: &Expr<()>, x: Expr<()>) -> Expr<()> {
    match s.expr_kind() {
        ExprKind::Set(elems) => contains_literal(elems, &x),
        _ => Expr::contains(s.clone(), x),
    }
}

/// `[l₁, …, lₙ].contains(x)`: `l₁ == x || … || lₙ == x`.
fn contains_literal(elems: &[Expr<()>], x: &Expr<()>) -> Expr<()> {
    or_chain(elems.iter().map(|l| eq(l, x)))
}

/// Drops the guards that `term` evaluates itself, first and in the same
/// order: the longest suffix of `guards` that is a prefix of [`eval_list`]
/// of `term`. Under those guards `term` evaluates exactly like on its own —
/// an erring guard is the first erring subterm of `term`, with the same
/// error (Lean `dropRepeated`, `underGuards_evalList_prefix`).
fn drop_repeated(mut guards: Vec<Expr<()>>, term: &Expr<()>) -> Vec<Expr<()>> {
    let mut order = Vec::new();
    eval_list(term, &mut order);
    let keep = (0..=guards.len())
        .find(|&keep| order.starts_with(guards.get(keep..).unwrap_or_default()))
        .unwrap_or(guards.len());
    guards.truncate(keep);
    guards
}

/// The subterms of `e` that can err, in evaluation order and at the
/// granularity of guards (a node that can err is a unit; one that never errs
/// by itself contributes its children's, in order), as far as evaluation is
/// certain to proceed: the test or left operand of an `if`/`&&`/`||` only,
/// after which the list is *incomplete* (`false`) — a strict parent lists
/// nothing past an incomplete child. An error in any of them is `e`'s
/// error, and it is the first one in this order.
fn eval_list(e: &Expr<()>, out: &mut Vec<Expr<()>>) -> bool {
    match e.expr_kind() {
        ExprKind::If { test_expr, .. } => {
            eval_list(test_expr, out);
            false
        }
        ExprKind::And { left, .. } | ExprKind::Or { left, .. } => {
            eval_list(left, out);
            false
        }
        _ if never_errs_itself(e) => children(e).into_iter().all(|child| eval_list(child, out)),
        _ => {
            out.push(e.clone());
            true
        }
    }
}
