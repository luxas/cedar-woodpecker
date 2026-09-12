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

//! Splitting atoms: hoisting `&&`/`||`/`!`/`if` nodes out of atoms so that
//! every atom becomes an opaque, indivisible term. See the module
//! documentation of [`super`] for the soundness argument.

use std::sync::Arc;

use cedar_policy_core::ast::{
    BinaryOp, Expr, ExprBuilder, ExprKind, Literal as AstLiteral, UnaryOp,
};
use cedar_policy_core::evaluator::stack_size_check;
use cedar_policy_core::expr_builder::ExprBuilder as _;

use super::{and_chain, bool_lit, conjuncts, disjuncts, DnfError};
use crate::expr_util::{children, erase, with_children, RebuildError};

/// Default node budget of [`split_atoms`].
pub const DEFAULT_MAX_SPLIT_NODES: usize = 65_536;

/// Rewrites the boolean expression `expr` so that no atom contains any
/// `&&`/`||`/`!`/`if` node, preserving Cedar's evaluation semantics exactly
/// — the same value, or the same error — on every input.
///
/// Within an atom every node is strict (all children are evaluated, left to
/// right, and the first error wins) and evaluation is pure, so the first
/// offending node `B` inside an atom `A[B]` can be hoisted to the top:
/// `A[if c then x else y]` becomes `if c then A[x] else A[y]`, and a boolean
/// `B` (`&&`/`||`/`!`) becomes `if B then A[true] else A[false]`. The hoisted
/// condition sits at structure position, where [`super::Dnf::of`] descends
/// into it natively; the substituted atoms are strictly smaller and are split
/// further until clean. An equality of two literals (`true == false`,
/// `1 == 3`), as the substitutions routinely produce inside `==`, is folded to
/// its boolean value (literal equality never errors).
///
/// Hoisting alone would *reorder* evaluation: the condition would run before
/// the atom's *left siblings* — the subterms Cedar evaluates before reaching
/// `B` — so where both a left sibling and the condition err, the two sides
/// would err differently. The hoisted `if` is therefore guarded by those
/// siblings, in evaluation order:
///
/// ```text
/// if (g₁ == g₁ && (g₂ == g₂ && …)) then (if c then A[x] else A[y]) else false
/// ```
///
/// `g == g` is `true` whenever `g` evaluates (equality is total value
/// equality, and reflexive) and reproduces `g`'s own error otherwise, so the
/// guarded form evaluates exactly like the original. Siblings that cannot
/// err — literals, variables, and set or record literals of such, after
/// folding — get no guard, and neither does a sibling already guarded by an
/// enclosing wrapper (the split of `A[x]` runs under `A`'s guards); without
/// guards there is no wrapper. The wrapper is an `if … else false` rather than `G && …` so that
/// a non-boolean atom value passes through unchanged (`&&` would turn it into
/// a type error), and the guards sit in the `if`'s test rather than in the
/// hoisted condition so that the DNF conversion does not emit a spurious
/// `!(g == g) && A[y]` cube per guard: the `else false` edge is a never-true
/// cube that the conversion prunes. In the cubes the guards appear as
/// `… && g == g && c && …` conjuncts, once each (Step 1's literal dedup);
/// the evaluator-guided pruning folds the never-erring ones away.
///
/// The one non-strict node inside atoms is `iferror(e, d)`, which *catches*
/// `e`'s error: hoisting a node out of it would move that node's error
/// outside the coalescing scope (`iferror(a && e, false)` is `false` where
/// `e` errors; `if (a && e) then … else …` errors). An `iferror` call is
/// therefore opaque — an atom whose inside is never split — and "clean" means
/// no offending node outside `iferror` calls and boolean equalities.
///
/// Splitting duplicates the atom around every hoisted node, so the result can
/// be exponentially larger than the input; `max_nodes` bounds the total
/// number of atom nodes processed, and exhausting it fails with
/// [`DnfError::TooLarge`]. The result is erased: the input's per-node data
/// cannot describe the substituted atom variants.
///
/// The typical pipeline for evaluator-guided pruning keeps this module
/// independent of the evaluator: `split_atoms` first, then
/// `Evaluator::evaluate` the split expression (dead branches fold away and
/// atoms get exact outcome sets), then `Dnf::of` with a `can_error` closure
/// reading that metadata.
pub fn split_atoms<T: Clone>(expr: &Expr<T>, max_nodes: usize) -> Result<Expr, DnfError> {
    split_structure(expr, &[], &mut Budget::new(max_nodes))
}

/// A node budget: [`split_atoms`]' atom nodes, the elimination's rewritten
/// atom nodes.
pub(super) struct Budget {
    remaining: usize,
    limit: usize,
}

impl Budget {
    pub(super) fn new(max_nodes: usize) -> Self {
        Budget {
            remaining: max_nodes,
            limit: max_nodes,
        }
    }

    pub(super) fn charge(&mut self, nodes: usize) -> Result<(), DnfError> {
        match self.remaining.checked_sub(nodes) {
            Some(rest) => {
                self.remaining = rest;
                Ok(())
            }
            None => Err(DnfError::TooLarge {
                limit: self.limit,
                what: "atom nodes",
            }),
        }
    }
}

/// Splits the atoms of a boolean expression, recursing through its
/// `&&`/`||`/`!`/`if` structure. `ctx` holds the subterms already known to
/// have evaluated wherever this expression is evaluated (the guards of
/// enclosing wrappers, the operands of enclosing `&&`/`||`/`if` nodes and
/// their strict subterms), so they need no guard here: inside the *then*
/// branch of `if c …` and the right operand of `l && r`, `c`/`l` was true
/// ([`learn_true`]); inside the *else* branch and the right operand of
/// `l || r`, it was false ([`learn_false`]).
fn split_structure<T: Clone>(
    expr: &Expr<T>,
    ctx: &[Expr<()>],
    budget: &mut Budget,
) -> Result<Expr, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    let node = |kind| ExprBuilder::new().with_expr_kind(kind);
    Ok(match expr.expr_kind() {
        ExprKind::Lit(AstLiteral::Bool(b)) => bool_lit(*b),
        ExprKind::And { left, right } => {
            let inner = extend(ctx, learn_true(left)?);
            node(ExprKind::And {
                left: Arc::new(split_structure(left, ctx, budget)?),
                right: Arc::new(split_structure(right, &inner, budget)?),
            })
        }
        ExprKind::Or { left, right } => {
            let inner = extend(ctx, learn_false(left)?);
            node(ExprKind::Or {
                left: Arc::new(split_structure(left, ctx, budget)?),
                right: Arc::new(split_structure(right, &inner, budget)?),
            })
        }
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => node(ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg: Arc::new(split_structure(arg, ctx, budget)?),
        }),
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => {
            let inner_then = extend(ctx, learn_true(test_expr)?);
            let inner_else = extend(ctx, learn_false(test_expr)?);
            node(ExprKind::If {
                test_expr: Arc::new(split_structure(test_expr, ctx, budget)?),
                then_expr: Arc::new(split_structure(then_expr, &inner_then, budget)?),
                else_expr: Arc::new(split_structure(else_expr, &inner_else, budget)?),
            })
        }
        _ => split_atom(expr, ctx, budget)?,
    })
}

/// The subterms `e` evaluates whenever it evaluates at all, to any value:
/// `e` itself, and recursively every child of a strict node, the test or
/// left operand of an `if`/`&&`/`||`, and nothing inside an `iferror` call
/// (which catches its argument's error) — without the nodes that never err
/// by themselves ([`never_errs_itself`]), which no guard ever is.
fn evaluated(e: &Expr<()>, out: &mut Vec<Expr<()>>) -> Result<(), DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    if !never_errs_itself(e) {
        out.push(e.clone());
    }
    if is_iferror_call(e) {
        return Ok(());
    }
    match e.expr_kind() {
        ExprKind::If { test_expr, .. } => evaluated(test_expr, out),
        ExprKind::And { left, .. } | ExprKind::Or { left, .. } => evaluated(left, out),
        _ => {
            for child in children(e) {
                evaluated(child, out)?;
            }
            Ok(())
        }
    }
}

/// What a `cond` that evaluated to a known value establishes: every member
/// of its `spine` — the `&&`-spine when it was `true`, the `||`-spine when
/// it was `false` — evaluated, and so did their strict subterms.
fn learn<T: Clone>(
    cond: &Expr<T>,
    spine: fn(&Expr) -> Vec<&Expr>,
) -> Result<Vec<Expr<()>>, DnfError> {
    let cond = erase(cond).map_err(rebuild_err)?;
    let mut out = Vec::new();
    for c in spine(&cond) {
        evaluated(c, &mut out)?;
    }
    Ok(out)
}

/// What a true `cond` establishes ([`learn`] over [`conjuncts`]).
pub(super) fn learn_true<T: Clone>(cond: &Expr<T>) -> Result<Vec<Expr<()>>, DnfError> {
    learn(cond, conjuncts)
}

/// What a false `cond` establishes ([`learn`] over [`disjuncts`]).
pub(super) fn learn_false<T: Clone>(cond: &Expr<T>) -> Result<Vec<Expr<()>>, DnfError> {
    learn(cond, disjuncts)
}

/// `ctx` extended by `learned`.
pub(super) fn extend(ctx: &[Expr<()>], learned: Vec<Expr<()>>) -> Vec<Expr<()>> {
    ctx.iter().cloned().chain(learned).collect()
}

/// Splits one atom: hoists its first offending node, if any, guarded by the
/// node's left siblings not already established by `ctx`, and recurses under
/// the extended context.
fn split_atom<T: Clone>(
    atom: &Expr<T>,
    ctx: &[Expr<()>],
    budget: &mut Budget,
) -> Result<Expr, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    budget.charge(atom.subexpressions().count())?;
    match hoist(atom)? {
        Hoist::Clean => erase_and_fold(atom),
        Hoist::Found {
            guards,
            cond,
            when_true,
            when_false,
        } => {
            let fresh = new_guards(ctx, guards);
            let inner = extend(ctx, fresh.clone());
            // The hoisted condition is at structure position now.
            let test = split_structure(&cond, &inner, budget)?;
            // It ran before either copy: its strict subterms need no guard
            // inside them.
            let mut copies = inner;
            evaluated(&cond, &mut copies)?;
            let hoisted = ExprBuilder::new().with_expr_kind(ExprKind::If {
                test_expr: Arc::new(test),
                then_expr: Arc::new(split_atom(&when_true, &copies, budget)?),
                else_expr: Arc::new(split_atom(&when_false, &copies, budget)?),
            });
            Ok(guard(fresh, hoisted))
        }
    }
}

/// The guards not already established by `ctx`, in order and without
/// repetition (a later duplicate is dropped).
pub(super) fn new_guards(ctx: &[Expr<()>], guards: Vec<Expr<()>>) -> Vec<Expr<()>> {
    let mut fresh: Vec<Expr<()>> = Vec::new();
    for g in guards {
        if !ctx.contains(&g) && !fresh.contains(&g) {
            fresh.push(g);
        }
    }
    fresh
}

/// Wraps `e` as `if (g₁ == g₁ && (g₂ == g₂ && …)) then e else false` over
/// the `guards`, in order — the left siblings whose errors must surface before
/// the hoisted condition's (see [`split_atoms`]); `e` itself without guards.
pub(super) fn guard(guards: Vec<Expr<()>>, e: Expr<()>) -> Expr<()> {
    if guards.is_empty() {
        return e;
    }
    ExprBuilder::new().with_expr_kind(ExprKind::If {
        test_expr: Arc::new(and_chain(guards.into_iter().map(self_eq))),
        then_expr: Arc::new(e),
        else_expr: Arc::new(bool_lit(false)),
    })
}

/// `g == g`: `true` whenever `g` evaluates, `g`'s error otherwise.
pub(super) fn self_eq(g: Expr<()>) -> Expr<()> {
    let g = Arc::new(g);
    ExprBuilder::new().with_expr_kind(ExprKind::BinaryApp {
        op: BinaryOp::Eq,
        arg1: g.clone(),
        arg2: g,
    })
}

/// Whether a node cannot err once its children have evaluated: a literal,
/// a variable, set and record construction, and `==` (total value
/// equality). Such a node needs no guard of its own; its children's guards
/// speak for it.
pub(super) fn never_errs_itself<T>(e: &Expr<T>) -> bool {
    matches!(
        e.expr_kind(),
        ExprKind::Lit(_)
            | ExprKind::Var(_)
            | ExprKind::Set(_)
            | ExprKind::Record(_)
            | ExprKind::BinaryApp {
                op: BinaryOp::Eq,
                ..
            }
    )
}

/// Appends the guards for a left sibling `e` (already erased and folded) to
/// `guards`: a node that never errs by itself contributes its children's
/// guards in order (so a literal or a variable contributes none); anything
/// else is a guard itself. A guard is a left sibling of the *first*
/// offender in evaluation order, so it holds no offender itself and its
/// decomposition is clean.
pub(super) fn add_guards(e: Expr<()>, guards: &mut Vec<Expr<()>>) {
    if never_errs_itself(&e) {
        for child in children(&e) {
            add_guards(child.clone(), guards);
        }
    } else {
        guards.push(e);
    }
}

/// The guards for the left siblings `siblings` of an offending node, in
/// order: each erased and folded (so a literal equality an earlier
/// substitution produced counts as the literal it is), then decomposed by
/// [`add_guards`].
fn guards_of<T: Clone>(siblings: &[&Expr<T>]) -> Result<Vec<Expr<()>>, DnfError> {
    let mut guards = Vec::new();
    for sibling in siblings {
        add_guards(erase_and_fold(sibling)?, &mut guards);
    }
    Ok(guards)
}

/// The result of looking for the first `&&`/`||`/`!`/`if` node (in pre-order,
/// left to right — evaluation order) strictly inside an expression.
enum Hoist {
    /// No offending node.
    Clean,
    /// An offending node was found: the whole expression evaluates like
    /// `if cond then when_true else when_false`, once every guard has
    /// evaluated without error.
    Found {
        /// The left siblings of the offending node (the subterms evaluated
        /// before it), in evaluation order, folded, without the ones that
        /// never err.
        guards: Vec<Expr<()>>,
        cond: Expr<()>,
        when_true: Expr<()>,
        when_false: Expr<()>,
    },
}

/// Whether `e` is boolean structure — an `&&`, `||`, `!` or `if` node — the
/// node kind the splitter hoists out of atoms and the elimination treats as
/// opaque inside one.
pub(super) fn is_offender<T>(e: &Expr<T>) -> bool {
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

/// Whether `e` is an `iferror(…)` call, which the splitter never looks inside.
pub(crate) fn is_iferror_call<T>(e: &Expr<T>) -> bool {
    matches!(
        e.expr_kind(),
        ExprKind::ExtensionFunctionApp { fn_name, .. }
            if cedar_policy_core::extensions::iferror::is_iferror(fn_name)
    )
}

fn hoist<T: Clone>(e: &Expr<T>) -> Result<Hoist, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    // `iferror(e, d)` is not strict: it *catches* `e`'s error. Hoisting a node
    // out of it would move that node's error outside the coalescing scope
    // (`iferror(a && e, false)` is `false` where `e` errors, but
    // `if (a && e) then … else …` errors), so an `iferror` call is opaque —
    // an atom whose inside is never split. (A boolean-node operand of `==`
    // is hoisted like any other offender: with the literal fold, `x == y`
    // derives the cube set `(x && y) || (!x && !y)` — plan 10; an earlier
    // experiment made them opaque atoms, since reverted — so no atom hides `&&`/`||`/`!`.)
    if is_iferror_call(e) {
        return Ok(Hoist::Clean);
    }
    // The children searched so far without finding an offender are evaluated
    // before it: they become its guards (erased only on a find — a clean
    // atom, the common case, pays nothing).
    let mut siblings: Vec<&Expr<T>> = Vec::new();
    for (i, child) in children(e).into_iter().enumerate() {
        match child.expr_kind() {
            ExprKind::If {
                test_expr,
                then_expr,
                else_expr,
            } => {
                return Ok(Hoist::Found {
                    guards: guards_of(&siblings)?,
                    cond: erase(test_expr).map_err(rebuild_err)?,
                    when_true: replace_child(e, i, erase(then_expr).map_err(rebuild_err)?)?,
                    when_false: replace_child(e, i, erase(else_expr).map_err(rebuild_err)?)?,
                })
            }
            _ if is_offender(child) => {
                return Ok(Hoist::Found {
                    guards: guards_of(&siblings)?,
                    cond: erase(child).map_err(rebuild_err)?,
                    when_true: replace_child(e, i, bool_lit(true))?,
                    when_false: replace_child(e, i, bool_lit(false))?,
                })
            }
            _ => {
                if let Hoist::Found {
                    guards: inner,
                    cond,
                    when_true,
                    when_false,
                } = hoist(child)?
                {
                    // The earlier siblings are evaluated before anything
                    // inside `child`.
                    let mut guards = guards_of(&siblings)?;
                    guards.extend(inner);
                    return Ok(Hoist::Found {
                        guards,
                        cond,
                        when_true: replace_child(e, i, when_true)?,
                        when_false: replace_child(e, i, when_false)?,
                    });
                }
                siblings.push(child);
            }
        }
    }
    Ok(Hoist::Clean)
}

/// Rebuilds `e` (erased) with its `index`th child replaced by `replacement`
/// and every other child erased unchanged.
fn replace_child<T: Clone>(
    e: &Expr<T>,
    index: usize,
    replacement: Expr<()>,
) -> Result<Expr<()>, DnfError> {
    let mut replacement = Some(replacement);
    let mut rebuilt = Vec::new();
    for (i, child) in children(e).into_iter().enumerate() {
        rebuilt.push(if i == index {
            #[expect(clippy::expect_used, reason = "each index is visited exactly once")]
            replacement.take().expect("child index visited twice")
        } else {
            erase(child).map_err(rebuild_err)?
        });
    }
    if replacement.is_some() {
        return Err(DnfError::Unsupported("child index out of range"));
    }
    with_children(e, rebuilt, ()).map_err(rebuild_err)
}

/// Maps a rebuild error into the DNF error space.
pub(super) fn rebuild_err(e: RebuildError) -> DnfError {
    match e {
        RebuildError::Unsupported(what) => DnfError::Unsupported(what),
        RebuildError::RecursionLimit => DnfError::RecursionLimit,
    }
}

/// Erases `e` and folds every equality of two literals to its boolean value
/// in one pass: literal equality is decided by total value equality across
/// types and never errors.
pub(super) fn erase_and_fold<T: Clone>(e: &Expr<T>) -> Result<Expr, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    let folded = children(e)
        .into_iter()
        .map(erase_and_fold)
        .collect::<Result<Vec<_>, _>>()?;
    let rebuilt = with_children(e, folded, ()).map_err(rebuild_err)?;
    if let ExprKind::BinaryApp {
        op: cedar_policy_core::ast::BinaryOp::Eq,
        arg1,
        arg2,
    } = rebuilt.expr_kind()
    {
        if let (ExprKind::Lit(a), ExprKind::Lit(b)) = (arg1.expr_kind(), arg2.expr_kind()) {
            return Ok(bool_lit(a == b));
        }
    }
    Ok(rebuilt)
}
