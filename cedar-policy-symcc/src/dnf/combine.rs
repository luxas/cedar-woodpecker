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

//! Combining a policy set's allow and deny policies into allow policies only
//! (Phase 3 Step 4, part 2). See the module documentation of [`super`] for
//! the argument.

use cedar_policy_core::ast::{Effect, Expr, Policy, PolicySet};

use super::policy::static_instance;
use super::{and_chain, bool_lit, conjuncts, split_policy_set, DnfError};

/// The conjuncts of a forbid's full condition: its three scope constraints
/// (with any template slot filled — `Policy::condition()` would render the
/// template's slot) and then its `when` clause, each flattened by
/// [`conjuncts`]; the Lean model's `conjuncts F.toExpr` for
/// `F.toExpr = pScope && (aScope && (rScope && when))`.
fn chain(f: &Policy) -> Vec<Expr> {
    let scope = [
        f.principal_constraint().as_expr(),
        f.action_constraint().as_expr(),
        f.resource_constraint().as_expr(),
    ];
    scope
        .iter()
        .chain(f.non_scope_constraints())
        .flat_map(|e| conjuncts(e).into_iter().cloned())
        .collect()
}

/// `!iferror(d, false)`: `true` exactly when `d` is not `true` (false or
/// erring), never erring for a boolean-or-error `d`.
fn not_true(d: &Expr) -> Expr {
    Expr::not(Expr::call_extension_fn(
        cedar_policy_core::extensions::iferror::IFERROR_NAME.clone(),
        vec![d.clone(), bool_lit(false)],
    ))
}

/// The witness that the `&&`-chain `ds` is *not* true, nested right:
/// `!iferror(d₁, false) || (d₁ && (!iferror(d₂, false) || (d₂ && … !iferror(dₖ, false))))`.
/// On any input on which every `dᵢ` is boolean-or-error it evaluates to a
/// boolean (never errs) — `true` exactly when `d₁ && … && dₖ` is not `true`,
/// which is when the first non-true conjunct exists: the disjunct for that
/// conjunct is the one that fires, after the guards `d₁ && … && d_{l-1}` of the
/// earlier conjuncts have all been found `true`. An empty chain is `true`, so
/// its witness is `false`.
pub fn deny_witness(ds: &[&Expr]) -> Expr {
    match ds {
        [] => bool_lit(false),
        [d] => not_true(d),
        [d, rest @ ..] => Expr::or(not_true(d), Expr::and((*d).clone(), deny_witness(rest))),
    }
}

/// Rewrites `policies` into **allow policies only** with the same
/// authorization decision on every input on which the forbids' conjuncts are
/// boolean-or-error (everything that validates). Every permit `P` becomes one
/// permit with `P`'s scope and annotations and the `when` condition
/// `when_P && W(F₁) && … && W(Fₘ)`, where `W(F)` is [`deny_witness`] over the
/// `&&`-spine of `F`'s full condition (scope included) and the forbids are
/// taken in order of their ids; the forbids themselves are dropped. A permit
/// without a `when` clause contributes no conjunct; without forbids every
/// permit is carried over unchanged (template-linked policies become
/// equivalent static ones, as in [`split_policy`](super::split_policy), and
/// a permit without a `when` clause gets `when { true }`); unlinked templates
/// are dropped.
///
/// A decision is `allow` exactly when some permit is `true` and no forbid is;
/// `W(F)` is `true` exactly when `F` is not, and it never errs where `F`'s
/// conjuncts are boolean-or-error, so `when_P && W(F₁) && …` is `true` exactly
/// when the original decision is `allow`. With no forbids left, that is the
/// combined set's decision. The Lean model `Cedar.DNF.combineAllowDeny`
/// proves this (`combineAllowDeny_decision`).
pub fn combine_allow_deny(policies: &PolicySet) -> Result<PolicySet, DnfError> {
    let mut forbids: Vec<&Policy> = policies
        .policies()
        .filter(|p| p.effect() == Effect::Forbid)
        .collect();
    forbids.sort_by(|a, b| a.id().as_ref().cmp(b.id().as_ref()));
    let witnesses: Vec<Expr> = forbids
        .iter()
        .map(|f| {
            let chain = chain(f);
            deny_witness(&chain.iter().collect::<Vec<_>>())
        })
        .collect();
    let mut out = PolicySet::new();
    for policy in policies.policies() {
        if policy.effect() != Effect::Permit {
            continue;
        }
        // a permit without a `when` clause (`None`, distinct from an explicit
        // `when { true }`) contributes no conjunct — the Lean model's
        // `Policy` has an empty condition list; nested right,
        // `when && (W₁ && (W₂ && …))`: the shape the model's
        // `Conditions.toExpr` gives the condition list, so the DRT compares
        // structurally
        let combined = and_chain(
            policy
                .non_scope_constraints()
                .cloned()
                .into_iter()
                .chain(witnesses.iter().cloned()),
        );
        let allow = static_instance(policy, policy.id().clone(), Effect::Permit, Some(combined))?;
        // Unreachable: the permits' ids are unique within the input set.
        out.add(allow)
            .map_err(|_| DnfError::Unsupported("duplicate policy id"))?;
    }
    Ok(out)
}

/// [`combine_allow_deny`] followed by [`split_policy_set`]: the allow-only
/// **cube** policies — one per DNF cube of each combined permit, i.e. the
/// allow/deny cross terms of the README's Step 4 (`a₁ && … && aₖ && d₁ && … &&
/// !iferror(dₗ, false) && …`, with Step 1's guard literals). Same decision as
/// `policies` on every input where the forbids' conjuncts are boolean-or-error;
/// the budgets bound the `O(k^m)` blow-up. The *combined* permits are what
/// is validated against `schema` (strict) — the hypothesis the Lean
/// `allowCubes_decision` carries — so a forbid is typed in the permit
/// environments it can reach, past the scope conjuncts; an ill-typed forbid
/// no permit environment reaches is not reported.
pub fn allow_cubes(
    policies: &PolicySet,
    schema: &cedar_policy::Schema,
    max_split_nodes: usize,
    max_cubes: usize,
) -> Result<PolicySet, DnfError> {
    split_policy_set(
        &combine_allow_deny(policies)?,
        schema,
        max_split_nodes,
        max_cubes,
    )
}
