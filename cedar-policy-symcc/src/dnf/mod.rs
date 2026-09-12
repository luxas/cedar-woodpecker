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

//! Disjunctive normal form of a boolean Cedar expression, respecting Cedar's
//! three-valued, short-circuiting evaluation semantics.
//!
//! See `evaluator/README.md`, Phase 3, for the roadmap. This module covers
//! Step 1: the `&&` / `||` / `!` / `if` structure of an expression is
//! converted; every other node is an *atom* — opaque, pure, and possibly
//! erroring — that is left as is.
//!
//! # Why the textbook rules do not apply
//!
//! Cedar's `&&` and `||` evaluate left to right and stop early, and every
//! boolean sub-expression may error. So `(a || b) && c` is **not**
//! `(a && c) || (b && c)`: for `a = true, c = false` the original never
//! evaluates `b`, which may error, while the rewrite does. The correct form is
//! `(a && c) || (!a && b && c)`.
//!
//! # Construction
//!
//! The converter linearises the evaluation *decision tree*: each atom is a node
//! whose `true` and `false` edges lead to what Cedar evaluates next (an error
//! ends the evaluation), and leaves are `true` / `false`. Every root-to-leaf
//! path is one *cube*: the literals along it, in evaluation order, negated on a
//! `false` edge. Paths to a `true` leaf are the cubes that can be true; paths
//! to a `false` leaf are *never-true* cubes, rendered with a trailing
//! `&& false`, that exist only to reproduce the errors of the atoms on them.
//!
//! On any input, the original evaluation follows exactly one path `Q`. A cube
//! evaluates its literals under the same conditions as the tree, so it is
//! `false` if it leaves `Q`, an error if it passes through the node where `Q`
//! errors, and `Q`'s own leaf if it is `Q`. Hence at most one cube is `true`,
//! error cubes only coexist with `false` ones, and the ordered `||` of the
//! cubes equals the original *in any cube order* and *for any subset of the
//! never-true cubes that still contains every node that can error*. This
//! justifies the two simplifications:
//!
//! * a literal whose atom already occurs earlier in its cube is determined by
//!   the prefix and dropped (same polarity) or turns the cube into a never-true
//!   one (opposite polarity);
//! * a never-true cube is kept only if it contains a node — an atom under a
//!   given prefix of literals — that may error and that no other kept cube
//!   contains. Which atoms may error is decided by the caller (see
//!   [`Dnf::of`]).
//!
//! The result is equivalent to the input on every input on which the
//! `can_error` answers are correct: for [`Dnf::of_expr`] that is every input;
//! for answers taken from the symbolic evaluator's outcome sets it is the
//! evaluator's own envelope (the inputs satisfying its assumptions).
//!
//! DNF is exponential in general; [`Dnf::of`] fails with
//! [`DnfError::TooLarge`] rather than exceeding a cube budget.
//!
//! # Splitting atoms (Step 2)
//!
//! An atom may hide boolean structure inside: an `if` at any position
//! (`(if c then x else y).field == "s"`), or a boolean `&&`/`||`/`!` under an
//! `==`, a set or a record literal. [`split_atoms`] hoists those out first,
//! so that afterwards no atom contains any `&&`/`||`/`!`/`if` node outside an
//! `iferror(…)` call (which is opaque: it catches its argument's error, so
//! nothing may be hoisted out of it), preserving evaluation exactly — the
//! hoisted `if` is guarded by `g == g` for the left siblings `g` of the
//! hoisted node, so their errors still surface first; see its documentation
//! for the construction and the strictness-and-purity argument for its
//! soundness. Splitting composes with the conversion:
//! `Dnf::of_expr(&split_atoms(&e, n)?)` yields cubes whose literals are
//! opaque, indivisible terms, the guards among them.

mod interpret;
mod like;
mod paths;
mod split;

use std::fmt;
use std::sync::Arc;

use cedar_policy_core::ast::{Expr, ExprBuilder, ExprKind, Literal as AstLiteral, UnaryOp};
use cedar_policy_core::expr_builder::ExprBuilder as _;
use miette::Diagnostic;
use thiserror::Error;

pub use interpret::interpret;
pub use like::{likes_have_wildcards, rewrite_like};
pub use split::{split_atoms, DEFAULT_MAX_SPLIT_NODES};

/// Default cube budget of [`Dnf::of_expr`]: the number of paths (cubes before
/// pruning) after which conversion fails with [`DnfError::TooLarge`].
pub const DEFAULT_MAX_CUBES: usize = 4096;

/// Errors of the DNF conversion.
#[derive(Debug, Clone, PartialEq, Eq, Diagnostic, Error)]
pub enum DnfError {
    /// The conversion would exceed its budget: cubes for [`Dnf::of`], atom
    /// nodes for [`split_atoms`].
    #[error("the conversion would exceed its budget of {limit} {what}")]
    TooLarge {
        /// The budget that was exceeded.
        limit: usize,
        /// What the budget counts: "cubes" or "atom nodes".
        what: &'static str,
    },
    /// The expression nests too deeply (same limit as the concrete evaluator).
    #[error("recursion limit reached while converting the expression")]
    RecursionLimit,
    /// The expression contains a node kind the converter does not handle.
    #[error("unsupported expression: {0}")]
    Unsupported(&'static str),
}

/// An atom or its negation.
#[derive(Debug, Clone)]
pub struct Literal<T> {
    atom: Arc<Expr<T>>,
    /// `atom` with its data erased: the identity of the atom.
    key: Expr<()>,
    negated: bool,
}

impl<T> Literal<T> {
    /// The atom (an expression that is not `&&`, `||`, `!`, `if` or a boolean
    /// literal), with the data it carried in the input.
    pub fn atom(&self) -> &Arc<Expr<T>> {
        &self.atom
    }

    /// Whether the literal is the negation of its atom.
    pub fn negated(&self) -> bool {
        self.negated
    }

    /// The literal as an expression: the atom, or `!atom`.
    pub fn to_expr(&self) -> Expr {
        if self.negated {
            ExprBuilder::new().with_expr_kind(ExprKind::UnaryApp {
                op: UnaryOp::Not,
                arg: Arc::new(self.key.clone()),
            })
        } else {
            self.key.clone()
        }
    }
}

impl<T> PartialEq for Literal<T> {
    /// Literals are equal when their atoms are structurally equal (ignoring
    /// data and source locations) with the same polarity.
    fn eq(&self, other: &Self) -> bool {
        self.negated == other.negated && self.key == other.key
    }
}

impl<T> Eq for Literal<T> {}

impl<T> fmt::Display for Literal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_expr())
    }
}

/// A conjunction of literals, evaluated left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cube<T> {
    literals: Vec<Literal<T>>,
    /// The cube ends in `&& false`: it is never true and exists only to
    /// reproduce the errors of its atoms.
    never_true: bool,
}

impl<T> Cube<T> {
    /// The literals, in evaluation order.
    pub fn literals(&self) -> &[Literal<T>] {
        &self.literals
    }

    /// Whether the cube can never be true (it ends in `&& false`).
    pub fn is_never_true(&self) -> bool {
        self.never_true
    }

    /// The cube as an expression: a left-associative `&&` chain of its
    /// literals, followed by `&& false` if it is never true; `true` if empty.
    pub fn to_expr(&self) -> Expr {
        let mut terms = self.literals.iter().map(Literal::to_expr);
        let first = match terms.next() {
            Some(first) => first,
            None => bool_lit(!self.never_true),
        };
        let mut acc = first;
        for term in terms {
            acc = and(acc, term);
        }
        if self.never_true && !self.literals.is_empty() {
            acc = and(acc, bool_lit(false));
        }
        acc
    }
}

impl<T> fmt::Display for Cube<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_expr())
    }
}

/// A disjunction of cubes, evaluated left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dnf<T> {
    cubes: Vec<Cube<T>>,
}

impl<T: Clone> Dnf<T> {
    /// Converts a boolean `expr`, asking `can_error` whether an atom may
    /// evaluate to an error. Answering `true` is always sound; answering
    /// `false` for an atom that can error on some input makes the result
    /// differ from `expr` (it is `false` where `expr` errors) on that input.
    /// `max_cubes` bounds the number of paths (cubes before pruning).
    pub fn of(
        expr: &Expr<T>,
        can_error: impl FnMut(&Expr<T>) -> bool,
        max_cubes: usize,
    ) -> Result<Self, DnfError> {
        let paths = paths::paths(expr, max_cubes)?;
        Ok(Self {
            cubes: paths::prune(paths, can_error),
        })
    }

    /// [`Self::of`] with every atom assumed able to error and the
    /// [`DEFAULT_MAX_CUBES`] budget. Equivalent to `expr` on every input.
    pub fn of_expr(expr: &Expr<T>) -> Result<Self, DnfError> {
        Self::of(expr, |_| true, DEFAULT_MAX_CUBES)
    }
}

impl<T> Dnf<T> {
    /// The cubes, in order.
    pub fn cubes(&self) -> &[Cube<T>] {
        &self.cubes
    }

    /// The cubes that can be true.
    pub fn true_cubes(&self) -> impl Iterator<Item = &Cube<T>> {
        self.cubes.iter().filter(|c| !c.never_true)
    }

    /// The DNF as an expression: a left-associative `||` chain of its cubes;
    /// `false` if there are none.
    pub fn to_expr(&self) -> Expr {
        let mut cubes = self.cubes.iter().map(Cube::to_expr);
        let mut acc = match cubes.next() {
            Some(first) => first,
            None => return bool_lit(false),
        };
        for cube in cubes {
            acc = ExprBuilder::new().with_expr_kind(ExprKind::Or {
                left: Arc::new(acc),
                right: Arc::new(cube),
            });
        }
        acc
    }
}

impl<T> fmt::Display for Dnf<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_expr())
    }
}

fn bool_lit(b: bool) -> Expr {
    ExprBuilder::new().with_expr_kind(ExprKind::Lit(AstLiteral::Bool(b)))
}

/// `left && right` without the constant folding `ExprBuilder::and` does.
fn and(left: Expr, right: Expr) -> Expr {
    ExprBuilder::new().with_expr_kind(ExprKind::And {
        left: Arc::new(left),
        right: Arc::new(right),
    })
}

/// `d₁ && (d₂ && …)`, nested right — the shape the Lean model's chains have,
/// so the DRT compares structurally; the empty chain is `true`.
fn and_chain(ds: impl DoubleEndedIterator<Item = Expr>) -> Expr {
    ds.rev()
        .reduce(|acc, d| and(d, acc))
        .unwrap_or_else(|| bool_lit(true))
}

/// The `&&`-spine of `e`, left to right, without `true` literals: `(a && b) && c`
/// and `a && (b && c)` both give `[a, b, c]`; anything else is a single conjunct
/// (an unconstrained scope renders as `true`; `true && x` is `x`).
pub fn conjuncts(e: &Expr) -> Vec<&Expr> {
    spine(e, true)
}

/// The `||`-spine of `e`, left to right, without `false` literals.
fn disjuncts(e: &Expr) -> Vec<&Expr> {
    spine(e, false)
}

/// The `&&`-spine (`and`) or `||`-spine of `e`, without its unit literal.
fn spine(e: &Expr, and: bool) -> Vec<&Expr> {
    fn go<'a>(e: &'a Expr, and: bool, out: &mut Vec<&'a Expr>) {
        match (and, e.expr_kind()) {
            (true, ExprKind::And { left, right }) | (false, ExprKind::Or { left, right }) => {
                go(left, and, out);
                go(right, and, out);
            }
            (_, ExprKind::Lit(AstLiteral::Bool(b))) if *b == and => {}
            _ => out.push(e),
        }
    }
    let mut out = Vec::new();
    go(e, and, &mut out);
    out
}
