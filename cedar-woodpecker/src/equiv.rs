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

//! Equivalence classes: what a path's equalities let the other conjuncts say
//! without the intermediate request.
//!
//! Over the conjuncts of a path cube, in the README's order (plan 16):
//!
//! 1. a conjunct `<expr> == <constant>` substitutes the constant for `<expr>`
//!    in every other conjunct (a record literal substituted under `.a` /
//!    `has a` is simplified, literal equalities fold);
//! 2. the error guards `<expr> == <expr>` and `true` are dropped (an
//!    over-approximation may drop them);
//! 3. a conjunct equating an expression free of the intermediate request
//!    with one mentioning it substitutes the free one for the other;
//! 4. a conjunct equating two expressions that both mention it makes the
//!    left one canonical.
//!
//! Each step runs to a fixpoint before the next, and the whole runs to a
//! fixpoint. Sound: under a path that is `true`, every equality's sides took
//! equal values, so replacing one by the other in a pure expression keeps
//! the value the path's evaluation gave it; dropping a conjunct weakens.
//! Terminating: a canonical never contains the term it replaces (such a pair
//! is skipped), so every applied substitution removes occurrences of a
//! non-canonical term, and a bound on the rounds guards the rest.

use cedar_policy_core::ast::{BinaryOp, Expr, ExprKind, Literal};
use cedar_policy_symcc::dnf::rewrite_like;
use smol_str::SmolStr;

use crate::expr::{is_constant, mentions_context_attrs, occurs, replace, simplify};

/// The two sides of a top-level equality.
fn equality(e: &Expr) -> Option<(&Expr, &Expr)> {
    match e.expr_kind() {
        ExprKind::BinaryApp {
            op: BinaryOp::Eq,
            arg1,
            arg2,
        } => Some((arg1, arg2)),
        _ => None,
    }
}

/// A conjunct that says nothing: `true`, or a guard `<expr> == <expr>`.
fn is_trivial(e: &Expr) -> bool {
    matches!(e.expr_kind(), ExprKind::Lit(Literal::Bool(true)))
        || equality(e).is_some_and(|(a, b)| a == b)
}

/// The substitution phases, in order.
#[derive(Clone, Copy)]
enum Phase {
    Constant,
    LeaveRemove,
    RemoveRemove,
}

/// The substitution `(from, to)` conjunct `i` offers in `phase`, if any.
fn offer<'a>(e: &'a Expr, phase: Phase, removed: &[SmolStr]) -> Option<(&'a Expr, &'a Expr)> {
    let (a, b) = equality(e)?;
    let mentions = |x: &Expr| mentions_context_attrs(x, removed);
    let pair = match phase {
        Phase::Constant => {
            if is_constant(b) && !is_constant(a) {
                (a, b)
            } else if is_constant(a) && !is_constant(b) {
                (b, a)
            } else {
                return None;
            }
        }
        Phase::LeaveRemove => {
            if !mentions(a) && mentions(b) {
                (b, a)
            } else if !mentions(b) && mentions(a) {
                (a, b)
            } else {
                return None;
            }
        }
        Phase::RemoveRemove => {
            if mentions(a) && mentions(b) {
                (b, a)
            } else {
                return None;
            }
        }
    };
    // a canonical containing the term it replaces would loop
    (!occurs(pair.0, pair.1)).then_some(pair)
}

/// Applies the first substitution of `phase` that changes some other
/// conjunct; whether one did.
fn step(conjuncts: &mut [Expr], phase: Phase, removed: &[SmolStr]) -> bool {
    for i in 0..conjuncts.len() {
        let Some((from, to)) = conjuncts
            .get(i)
            .and_then(|e| offer(e, phase, removed))
            .map(|(f, t)| (f.clone(), t.clone()))
        else {
            continue;
        };
        let mut changed = false;
        for (j, c) in conjuncts.iter_mut().enumerate() {
            if j == i {
                continue;
            }
            let new = simplify(&replace(c, &from, &to));
            if new != *c {
                *c = new;
                changed = true;
            }
        }
        if changed {
            return true;
        }
    }
    false
}

/// The equivalence-class rewrite of a conjunction: the conjuncts after
/// constant propagation, guard removal and canonicalization, with the terms
/// mentioning the intermediate request (an attribute in `removed`)
/// substituted away wherever an equality allows.
pub fn rewrite(conjuncts: &[Expr], removed: &[SmolStr]) -> Vec<Expr> {
    let mut cs: Vec<Expr> = conjuncts
        .iter()
        .map(|c| simplify(&rewrite_like(c).unwrap_or_else(|_| c.clone())))
        .collect();
    let bound = cs.len() * cs.len() + 1;
    for _ in 0..bound {
        cs.retain(|c| !is_trivial(c));
        let changed = [Phase::Constant, Phase::LeaveRemove, Phase::RemoveRemove]
            .into_iter()
            .any(|phase| step(&mut cs, phase, removed));
        if !changed {
            break;
        }
    }
    cs.retain(|c| !is_trivial(c));
    cs
}
