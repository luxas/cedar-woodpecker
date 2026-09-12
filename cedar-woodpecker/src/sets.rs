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

//! Set and `in` rewrites: two conjuncts that use the same intermediate set
//! (or the same intermediate element of a set) in set operations become one
//! conjunct over the remaining operands, by the quantifier-elimination matrix
//! of plan 13 (`cedar-lean/Cedar/Thm/DNF/SetQE.lean` in cedar-spec, one cell per pair of
//! literals over the quantified `X`); two conjuncts that chain an intermediate
//! entity through `in` become one by transitivity.
//!
//! Sound in every cell: the result is *implied* by the pair with no domain
//! hypothesis (the hypotheses of the matrix are for the converse), so the
//! synthesized policy is never strengthened. The three inexpressible cells
//! use their proved over-approximations, with the literal `[e]` where Cedar
//! can state it. Pairs are taken best first: an informative exact result
//! before one exact under a hypothesis, before an over-approximation, before
//! `true`. The `in` facts are a closure over every `in` conjunct of a term,
//! taken first ([`in_closure`]).

use cedar_policy_core::ast::{BinaryOp, Expr, ExprKind, UnaryOp, Var};
use smol_str::SmolStr;

use crate::expr::{mentions_context_attrs, simplify};

/// A set literal over the quantified term `X`; the payload is the other
/// operand, free of the intermediate request.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// `X.isEmpty()`
    Empty,
    /// `X == A`
    Eq(Expr),
    /// `X.contains(e)`
    Contains(Expr),
    /// `A.contains(x)` — `x` the quantified element
    Member(Expr),
    /// `A.containsAll(X)`: `X ⊆ A`
    Sub(Expr),
    /// `X.containsAll(A)`: `A ⊆ X`
    Sup(Expr),
    /// `X.containsAny(A)` (either side)
    Inter(Expr),
    /// `X in c`
    InLeft(Expr),
    /// `a in X`
    InRight(Expr),
}

#[derive(Debug, Clone)]
struct Atom {
    x: Expr,
    kind: Kind,
    neg: bool,
}

/// How good a rewrite is; lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// Exact, informative.
    Exact,
    /// Exact under a domain hypothesis, informative.
    ExactH,
    /// An over-approximation, informative.
    Over,
    /// `true`: exact but uninformative.
    True,
}

/// `e` read as a set literal over an intermediate term, if it is one.
fn atom(e: &Expr, removed: &[SmolStr]) -> Option<Atom> {
    let m = |x: &Expr| mentions_context_attrs(x, removed);
    let (e, neg) = match e.expr_kind() {
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => (arg.as_ref(), true),
        _ => (e, false),
    };
    let (x, kind) = match e.expr_kind() {
        ExprKind::UnaryApp {
            op: UnaryOp::IsEmpty,
            arg,
        } if m(arg) => (arg.as_ref().clone(), Kind::Empty),
        ExprKind::BinaryApp { op, arg1, arg2 } => {
            let (a, b) = (arg1.as_ref(), arg2.as_ref());
            match (op, m(a), m(b)) {
                (BinaryOp::Eq, true, false) => (a.clone(), Kind::Eq(b.clone())),
                (BinaryOp::Eq, false, true) => (b.clone(), Kind::Eq(a.clone())),
                (BinaryOp::Contains, true, false) => (a.clone(), Kind::Contains(b.clone())),
                (BinaryOp::Contains, false, true) => (b.clone(), Kind::Member(a.clone())),
                // `a.containsAll(b)`: `b ⊆ a`
                (BinaryOp::ContainsAll, false, true) => (b.clone(), Kind::Sub(a.clone())),
                (BinaryOp::ContainsAll, true, false) => (a.clone(), Kind::Sup(b.clone())),
                (BinaryOp::ContainsAny, true, false) => (a.clone(), Kind::Inter(b.clone())),
                (BinaryOp::ContainsAny, false, true) => (b.clone(), Kind::Inter(a.clone())),
                (BinaryOp::In, true, false) => (a.clone(), Kind::InLeft(b.clone())),
                (BinaryOp::In, false, true) => (b.clone(), Kind::InRight(a.clone())),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(Atom { x, kind, neg })
}

fn not(e: Expr) -> Expr {
    Expr::not(e)
}
fn is_empty(a: &Expr) -> Expr {
    Expr::unary_app(UnaryOp::IsEmpty, a.clone())
}
fn contains(a: &Expr, e: &Expr) -> Expr {
    Expr::binary_app(BinaryOp::Contains, a.clone(), e.clone())
}
/// `a.containsAll(b)`: `b ⊆ a`.
fn contains_all(a: &Expr, b: &Expr) -> Expr {
    Expr::binary_app(BinaryOp::ContainsAll, a.clone(), b.clone())
}
fn contains_any(a: &Expr, b: &Expr) -> Expr {
    Expr::binary_app(BinaryOp::ContainsAny, a.clone(), b.clone())
}
fn eq(a: &Expr, b: &Expr) -> Expr {
    Expr::is_eq(a.clone(), b.clone())
}
fn is_in(a: &Expr, b: &Expr) -> Expr {
    Expr::is_in(a.clone(), b.clone())
}
fn set_of(e: &Expr) -> Expr {
    Expr::set([e.clone()])
}
fn and(a: Expr, b: Expr) -> Expr {
    Expr::and(a, b)
}
fn or(a: Expr, b: Expr) -> Expr {
    Expr::or(a, b)
}

/// The matrix cell of `(l, r)` in this order; the caller retries the other
/// order. Cell numbers refer to plan 13's table. `A`/`e` are `l`'s operand,
/// `B`/`f` are `r`'s.
fn cell(l: &Atom, r: &Atom) -> Option<(Expr, Rank)> {
    use Kind::*;
    let t = || Some((Expr::val(true), Rank::True));
    let f = || Some((Expr::val(false), Rank::Exact));
    let ex = |e: Expr| Some((e, Rank::Exact));
    let exh = |e: Expr| Some((e, Rank::ExactH));
    let over = |e: Expr| Some((e, Rank::Over));
    match (&l.kind, l.neg, &r.kind, r.neg) {
        // row E: X = ∅ (#1–12)
        (Empty, false, Empty, false) => t(),
        (Empty, false, Empty, true) => f(),
        (Empty, false, Eq(a), false) => ex(is_empty(a)),
        (Empty, false, Eq(a), true) => ex(not(is_empty(a))),
        (Empty, false, Contains(_), false) => f(),
        (Empty, false, Contains(_), true) => t(),
        (Empty, false, Sub(_), false) => t(),
        (Empty, false, Sub(_), true) => f(),
        (Empty, false, Sup(a), false) => ex(is_empty(a)),
        (Empty, false, Sup(a), true) => ex(not(is_empty(a))),
        (Empty, false, Inter(_), false) => f(),
        (Empty, false, Inter(_), true) => t(),
        // row ¬E: X ≠ ∅ (#13–23)
        (Empty, true, Empty, true) => t(),
        (Empty, true, Eq(a), false) => ex(not(is_empty(a))),
        (Empty, true, Eq(_), true) => t(),
        (Empty, true, Contains(_), false) => t(),
        (Empty, true, Contains(_), true) => t(),
        (Empty, true, Sub(a), false) => ex(not(is_empty(a))),
        (Empty, true, Sub(_), true) => t(),
        (Empty, true, Sup(_), false) => t(),
        (Empty, true, Sup(a), true) => exh(not(is_empty(a))),
        (Empty, true, Inter(a), false) => ex(not(is_empty(a))),
        (Empty, true, Inter(_), true) => t(),
        // row Q: X = A (#24–33)
        (Eq(a), false, Eq(b), false) => ex(eq(a, b)),
        (Eq(a), false, Eq(b), true) => ex(not(eq(a, b))),
        (Eq(a), false, Contains(e), false) => ex(contains(a, e)),
        (Eq(a), false, Contains(e), true) => ex(not(contains(a, e))),
        (Eq(a), false, Sub(b), false) => ex(contains_all(b, a)),
        (Eq(a), false, Sub(b), true) => ex(not(contains_all(b, a))),
        (Eq(a), false, Sup(b), false) => ex(contains_all(a, b)),
        (Eq(a), false, Sup(b), true) => ex(not(contains_all(a, b))),
        (Eq(a), false, Inter(b), false) => ex(contains_any(a, b)),
        (Eq(a), false, Inter(b), true) => ex(not(contains_any(a, b))),
        // row ¬Q: X ≠ A (#34–42)
        (Eq(_), true, Eq(_), true) => t(),
        (Eq(_), true, Contains(_), false) => t(),
        (Eq(_), true, Contains(_), true) => t(),
        (Eq(a), true, Sub(b), false) => ex(or(not(is_empty(a)), not(is_empty(b)))),
        (Eq(_), true, Sub(_), true) => t(),
        (Eq(_), true, Sup(_), false) => t(),
        (Eq(_), true, Sup(b), true) => exh(not(is_empty(b))),
        (Eq(_), true, Inter(b), false) => exh(not(is_empty(b))),
        (Eq(_), true, Inter(_), true) => t(),
        // row C: e ∈ X (#43–50)
        (Contains(_), false, Contains(_), false) => t(),
        (Contains(e), false, Contains(f), true) => ex(not(eq(e, f))),
        (Contains(e), false, Sub(a), false) => ex(contains(a, e)),
        (Contains(_), false, Sub(_), true) => t(),
        (Contains(_), false, Sup(_), false) => t(),
        (Contains(e), false, Sup(a), true) => ex(not(contains_all(&set_of(e), a))),
        (Contains(_), false, Inter(a), false) => ex(not(is_empty(a))),
        (Contains(e), false, Inter(a), true) => ex(not(contains(a, e))),
        // row ¬C: e ∉ X (#51–57)
        (Contains(_), true, Contains(_), true) => t(),
        (Contains(_), true, Sub(_), false) => t(),
        (Contains(_), true, Sub(_), true) => t(),
        (Contains(e), true, Sup(a), false) => ex(not(contains(a, e))),
        (Contains(_), true, Sup(a), true) => ex(not(is_empty(a))),
        (Contains(e), true, Inter(a), false) => ex(not(contains_all(&set_of(e), a))),
        (Contains(_), true, Inter(_), true) => t(),
        // row Sub: X ⊆ A (#58–63)
        (Sub(_), false, Sub(_), false) => t(),
        (Sub(a), false, Sub(b), true) => ex(not(contains_all(b, a))),
        (Sub(a), false, Sup(b), false) => ex(contains_all(a, b)),
        (Sub(_), false, Sup(b), true) => ex(not(is_empty(b))),
        (Sub(a), false, Inter(b), false) => ex(contains_any(a, b)),
        (Sub(_), false, Inter(_), true) => t(),
        // row ¬Sub: X ⊄ A (#64–68)
        (Sub(_), true, Sub(_), true) => t(),
        (Sub(_), true, Sup(_), false) => t(),
        (Sub(_), true, Sup(b), true) => exh(not(is_empty(b))),
        (Sub(_), true, Inter(b), false) => exh(not(is_empty(b))),
        (Sub(_), true, Inter(_), true) => t(),
        // row Sup: A ⊆ X (#69–72)
        (Sup(_), false, Sup(_), false) => t(),
        (Sup(a), false, Sup(b), true) => ex(not(contains_all(a, b))),
        (Sup(_), false, Inter(b), false) => ex(not(is_empty(b))),
        (Sup(a), false, Inter(b), true) => ex(not(contains_any(a, b))),
        // row ¬Sup: A ⊄ X (#73–75)
        (Sup(a), true, Sup(b), true) => ex(and(not(is_empty(a)), not(is_empty(b)))),
        (Sup(a), true, Inter(b), false) => over(and(not(is_empty(a)), not(is_empty(b)))),
        (Sup(a), true, Inter(_), true) => ex(not(is_empty(a))),
        // row I: X ∩ A ≠ ∅ (#76–78)
        (Inter(a), false, Inter(b), false) => ex(and(not(is_empty(a)), not(is_empty(b)))),
        (Inter(a), false, Inter(b), true) => ex(not(contains_all(b, a))),
        (Inter(_), true, Inter(_), true) => t(),
        // the element quantified: x ∈ A (#79–81)
        (Member(a), false, Member(b), false) => ex(contains_any(a, b)),
        (Member(a), false, Member(b), true) => ex(not(contains_all(b, a))),
        (Member(_), true, Member(_), true) => t(),
        _ => None,
    }
}

/// The rewrite of the pair: the better of the two orders.
fn combine(l: &Atom, r: &Atom) -> Option<(Expr, Rank)> {
    match (cell(l, r), cell(r, l)) {
        (Some(a), Some(b)) => Some(if b.1 < a.1 { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Whether `e` is an entity by syntax: `principal`, `resource`, or an entity
/// literal. (An attribute may be a set of entities, which `in` accepts on its
/// right but not on its left.)
fn is_entity(e: &Expr) -> bool {
    matches!(
        e.expr_kind(),
        ExprKind::Var(Var::Principal | Var::Resource)
            | ExprKind::Lit(cedar_policy_core::ast::Literal::EntityUID(_))
    )
}

/// The `in` closure: for every intermediate term used as an `in` operand,
/// every fact transitivity gives over the other operands — `a in X ∧ X in c
/// ⇒ a in c`; `X in a ∧ ¬(X in b) ⇒ ¬(a in b)` when `a` is an entity (on the
/// left of an `in` a set is a type error); `a in X ∧ ¬(b in X) ⇒ ¬(b in a)`
/// — and then the `in` conjuncts over that term are dropped (what they say
/// beyond those facts is the term's own, which goes with it).
fn in_closure(conjuncts: &[Expr], removed: &[SmolStr]) -> Vec<Expr> {
    let atoms: Vec<Option<Atom>> = conjuncts.iter().map(|c| atom(c, removed)).collect();
    let is_in_atom = |a: &Atom| matches!(a.kind, Kind::InLeft(_) | Kind::InRight(_));
    let mut out: Vec<Expr> = Vec::new();
    let mut done: Vec<Expr> = Vec::new();
    for (c, a) in conjuncts.iter().zip(&atoms) {
        let Some(a) = a.as_ref().filter(|a| is_in_atom(a)) else {
            out.push(c.clone());
            continue;
        };
        if done.contains(&a.x) {
            continue;
        }
        done.push(a.x.clone());
        let group: Vec<&Atom> = atoms
            .iter()
            .flatten()
            .filter(|b| is_in_atom(b) && b.x == a.x)
            .collect();
        for l in &group {
            for r in &group {
                let fact = match (&l.kind, l.neg, &r.kind, r.neg) {
                    (Kind::InRight(a), false, Kind::InLeft(c), false) => is_in(a, c),
                    (Kind::InLeft(a), false, Kind::InLeft(b), true) if is_entity(a) => {
                        not(is_in(a, b))
                    }
                    (Kind::InRight(a), false, Kind::InRight(b), true) => not(is_in(b, a)),
                    _ => continue,
                };
                if !out.contains(&fact) {
                    out.push(fact);
                }
            }
        }
    }
    out
}

/// The set and `in` rewrite of a conjunction: pairs of conjuncts over the
/// same intermediate term (an operand mentioning an attribute in `removed`,
/// the other operands free of them) are replaced by their cell's result,
/// best-ranked pair first, until none is left.
pub fn rewrite(conjuncts: &[Expr], removed: &[SmolStr]) -> Vec<Expr> {
    let mut cs: Vec<Expr> = in_closure(conjuncts, removed);
    loop {
        let atoms: Vec<Option<Atom>> = cs.iter().map(|c| atom(c, removed)).collect();
        let mut best: Option<(Rank, usize, usize, Expr)> = None;
        for (i, a) in atoms.iter().enumerate() {
            let Some(a) = a else { continue };
            for (j, b) in atoms.iter().enumerate().skip(i + 1) {
                let Some(b) = b else { continue };
                if a.x != b.x {
                    continue;
                }
                if let Some((e, rank)) = combine(a, b) {
                    if best.as_ref().is_none_or(|(r, ..)| rank < *r) {
                        best = Some((rank, i, j, e));
                    }
                }
            }
        }
        let Some((_, i, j, e)) = best else { break };
        cs.remove(j);
        cs.remove(i);
        let e = simplify(&e);
        if !matches!(
            e.expr_kind(),
            ExprKind::Lit(cedar_policy_core::ast::Literal::Bool(true))
        ) {
            cs.insert(i, e);
        }
    }
    cs
}
