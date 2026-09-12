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

//! Phase 4 Step 1: `x like "exact"` is `x == "exact"`.
//!
//! A `like` whose pattern has no wildcard matches exactly one string, so it
//! is an equality in disguise; rewriting it lets the string atoms of a cube
//! be either equalities (which feed equivalence classes) or genuine pattern
//! constraints — every `like` left has at least one wildcard, and so is
//! matched by at least two strings. The Lean model `Cedar.DNF.rewriteLike`
//! proves the rewrite sound (`evaluate_rewriteLike`) and complete
//! (`rewriteLike_complete`, `star_matches_two`).
//!
//! Soundness needs the operand to be a string or to error: `like` on a
//! non-string *value* is a type error, while `==` against a string literal
//! is `false`. Validated expressions satisfy this (the validator types the
//! operand of `like` as `String`); the rewrite itself is purely syntactic.

use cedar_policy_core::ast::{Expr, ExprBuilder, ExprKind, Literal, PatternElem};
use cedar_policy_core::expr_builder::ExprBuilder as _;
use smol_str::SmolStr;

use super::split::rebuild_err;
use super::DnfError;
use crate::expr_util::{children, with_children};

/// The string a wildcard-free pattern matches, if the pattern is one.
fn exact_string(elems: &[PatternElem]) -> Option<SmolStr> {
    elems
        .iter()
        .map(|e| match e {
            PatternElem::Char(c) => Some(*c),
            PatternElem::Wildcard => None,
        })
        .collect()
}

/// Rewrites every `x like p` whose pattern `p` has no wildcard into
/// `x == "<p>"`, bottom-up; everything else is unchanged (source locations
/// included).
pub fn rewrite_like(e: &Expr) -> Result<Expr, DnfError> {
    let kids = children(e)
        .into_iter()
        .map(rewrite_like)
        .collect::<Result<Vec<_>, _>>()?;
    let rebuilt = with_children(e, kids, ()).map_err(rebuild_err)?;
    if let ExprKind::Like { expr, pattern } = rebuilt.expr_kind() {
        if let Some(s) = exact_string(pattern.get_elems()) {
            return Ok(ExprBuilder::new()
                .with_same_source_loc(&rebuilt)
                .is_eq(expr.as_ref().clone(), Expr::val(Literal::String(s))));
        }
    }
    Ok(rebuilt)
}

/// Whether every `like` in `e` has at least one wildcard — what
/// [`rewrite_like`] guarantees of its result.
pub fn likes_have_wildcards(e: &Expr) -> bool {
    e.subexpressions().all(|x| match x.expr_kind() {
        ExprKind::Like { pattern, .. } => pattern
            .iter()
            .any(|elem| matches!(elem, PatternElem::Wildcard)),
        _ => true,
    })
}
