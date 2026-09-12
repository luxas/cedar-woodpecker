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

//! The paths of the evaluation decision tree, and the pruning of the
//! never-true ones. See the module documentation of [`super`].

use std::collections::HashSet;
use std::sync::Arc;

use cedar_policy_core::ast::{Expr, ExprKind, Literal as AstLiteral, UnaryOp};
use cedar_policy_core::evaluator::stack_size_check;

use super::{Cube, DnfError, Literal};
use crate::expr_util::erase;

/// Where a path ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    True,
    False,
    /// The path reached a literal contradicting an earlier one; whatever the
    /// tree does after that point is unreachable.
    Contradiction,
}

/// A root-to-leaf path of the decision tree: the literals in evaluation
/// order, with repeated atoms removed.
#[derive(Debug, Clone)]
pub(super) struct Path<T> {
    literals: Vec<Literal<T>>,
    leaf: Leaf,
}

impl<T: Clone> Path<T> {
    fn leaf(leaf: bool) -> Self {
        Self {
            literals: Vec::new(),
            leaf: if leaf { Leaf::True } else { Leaf::False },
        }
    }

    fn flipped(mut self) -> Self {
        self.leaf = match self.leaf {
            Leaf::True => Leaf::False,
            Leaf::False => Leaf::True,
            Leaf::Contradiction => Leaf::Contradiction,
        };
        self
    }

    /// `self` continued by `rest`, dropping literals of `rest` whose atom
    /// `self` already determined and stopping at the first contradiction.
    fn extended(&self, rest: &Path<T>) -> Self {
        let mut literals = self.literals.clone();
        let mut leaf = rest.leaf;
        for lit in &rest.literals {
            match literals.iter().find(|l| l.key == lit.key) {
                None => literals.push(lit.clone()),
                Some(seen) if seen.negated == lit.negated => {}
                Some(_) => {
                    leaf = Leaf::Contradiction;
                    break;
                }
            }
        }
        Self { literals, leaf }
    }

    /// The nodes the path visits, in order: for each literal, the prefix
    /// conditions under which the tree evaluates its atom together with that
    /// atom — the node's identity.
    fn nodes(&self) -> impl Iterator<Item = (Node, &Literal<T>)> {
        let mut prefix = Vec::new();
        self.literals.iter().map(move |l| {
            let node = Node {
                prefix: prefix.clone(),
                atom: l.key.clone(),
            };
            prefix.push((l.key.clone(), l.negated));
            (node, l)
        })
    }
}

/// A node of the decision tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Node {
    prefix: Vec<(Expr<()>, bool)>,
    atom: Expr<()>,
}

/// The paths of `expr`'s decision tree in depth-first order, `true` edge
/// first; fails when there would be more than `max_cubes` of them.
pub(super) fn paths<T: Clone>(expr: &Expr<T>, max_cubes: usize) -> Result<Vec<Path<T>>, DnfError> {
    stack_size_check().map_err(|_| DnfError::RecursionLimit)?;
    let too_large = |paths: Vec<Path<T>>| {
        if paths.len() > max_cubes {
            Err(DnfError::TooLarge {
                limit: max_cubes,
                what: "cubes",
            })
        } else {
            Ok(paths)
        }
    };
    match expr.expr_kind() {
        ExprKind::Lit(AstLiteral::Bool(b)) => Ok(vec![Path::leaf(*b)]),
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => Ok(paths(arg, max_cubes)?
            .into_iter()
            .map(Path::flipped)
            .collect()),
        ExprKind::And { left, right } => {
            let l = paths(left, max_cubes)?;
            too_large(graft(l, Leaf::True, || paths(right, max_cubes))?)
        }
        ExprKind::Or { left, right } => {
            let l = paths(left, max_cubes)?;
            too_large(graft(l, Leaf::False, || paths(right, max_cubes))?)
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => {
            let c = paths(test_expr, max_cubes)?;
            // Both branches are grafted in one pass: grafting the else
            // branch after the then branch would also extend the `false`
            // leaves the then branch brought in.
            let then_paths = if c.iter().any(|p| p.leaf == Leaf::True) {
                paths(then_expr, max_cubes)?
            } else {
                Vec::new()
            };
            let else_paths = if c.iter().any(|p| p.leaf == Leaf::False) {
                paths(else_expr, max_cubes)?
            } else {
                Vec::new()
            };
            let mut out = Vec::new();
            for p in c {
                match p.leaf {
                    Leaf::True => out.extend(then_paths.iter().map(|s| p.extended(s))),
                    Leaf::False => out.extend(else_paths.iter().map(|s| p.extended(s))),
                    Leaf::Contradiction => out.push(p),
                }
            }
            too_large(out)
        }
        _ => {
            let key = erase(expr).map_err(super::split::rebuild_err)?;
            let lit = |negated| Literal {
                atom: Arc::new(expr.clone()),
                key: key.clone(),
                negated,
            };
            Ok(vec![
                Path {
                    literals: vec![lit(false)],
                    leaf: Leaf::True,
                },
                Path {
                    literals: vec![lit(true)],
                    leaf: Leaf::False,
                },
            ])
        }
    }
}

/// Replaces every `at` leaf of `paths` by the paths of `sub`, which is only
/// computed if there is such a leaf. Other paths are kept in place.
fn graft<T: Clone>(
    paths: Vec<Path<T>>,
    at: Leaf,
    sub: impl FnOnce() -> Result<Vec<Path<T>>, DnfError>,
) -> Result<Vec<Path<T>>, DnfError> {
    if !paths.iter().any(|p| p.leaf == at) {
        return Ok(paths);
    }
    let sub = sub()?;
    let mut out = Vec::with_capacity(paths.len() + sub.len());
    for p in paths {
        if p.leaf == at {
            out.extend(sub.iter().map(|s| p.extended(s)));
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

/// Turns paths into cubes, dropping the never-true ones that reproduce no
/// error the remaining cubes do not already reproduce: every node of a
/// may-be-true cube is covered first; then, in order, a never-true path is
/// kept iff it has a node whose atom `can_error` and that is not yet covered.
pub(super) fn prune<T: Clone>(
    paths: Vec<Path<T>>,
    mut can_error: impl FnMut(&Expr<T>) -> bool,
) -> Vec<Cube<T>> {
    let mut covered: HashSet<Node> = HashSet::new();
    for p in paths.iter().filter(|p| p.leaf == Leaf::True) {
        covered.extend(p.nodes().map(|(n, _)| n));
    }
    let mut cubes = Vec::new();
    for p in paths {
        let never_true = p.leaf != Leaf::True;
        if never_true {
            let uncovered = p
                .nodes()
                .filter(|(n, l)| !covered.contains(n) && can_error(&l.atom))
                .map(|(n, _)| n)
                .collect::<Vec<_>>();
            if uncovered.is_empty() {
                continue;
            }
            covered.extend(uncovered);
        }
        cubes.push(Cube {
            literals: p.literals,
            never_true,
        });
    }
    cubes
}
