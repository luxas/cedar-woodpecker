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

//! Splitting a policy into one policy per cube of the DNF of its condition.
//! See the module documentation of [`super`] for the decision-preservation
//! argument.

use cedar_policy::Schema;
use cedar_policy_core::ast::{Effect, Expr, Policy, PolicyID, PolicySet, StaticPolicy};
use cedar_policy_core::validator::{ValidationMode, Validator};

use super::elim::normalize_unchecked;
use super::{bool_lit, Dnf, DnfError};

/// Splits `policy` into one static policy per cube of the DNF of its
/// condition, preserving its set of matching inputs exactly.
///
/// The `when` condition is piped through [`split_atoms`] and [`Dnf::of`], and
/// every cube that can be true becomes its own policy with the same effect,
/// scope and annotations, the id `{id}.cube{i}`, and the cube as its `when`
/// condition. The scope stays in place: the condition is `scope && when`,
/// which is `true` exactly when both parts are (the three-valued `&&` table),
/// so splitting only the `when` part and copying the scope preserves each
/// policy's matching set. Template-linked policies split into equivalent
/// static policies, because their scope constraints carry the filled slot
/// values.
///
/// On any input, at most one of the returned policies matches, and one of
/// them matches exactly when `policy` matches — so the authorization decision
/// of any policy set is unchanged by replacing `policy` with the returned
/// policies (a decision depends only on which policies match; see
/// [`split_policy_set`]). Never-true cubes are dropped for the same reason: a
/// policy whose condition errors or is false is ignored either way. The one
/// observable difference is diagnostics: on an input on which `policy`
/// errors, the number of erroring split policies can differ in either
/// direction — a dropped never-true cube's error disappears (`e && false`
/// splits into nothing), while cubes sharing an erroring prefix each error
/// (`e && (a || b)` splits into `e && a` and `e && !a && b`, two errors for
/// one). On an input on which `policy` does not error, no split policy does.
///
/// The condition goes through the normalization pipeline
/// ([`normalize_atoms`](super::normalize_atoms): split the atoms, eliminate
/// record and set literals, split again) before the conversion, whose
/// rewrites assume a well-typed input: the policy is validated against
/// `schema` first (strict mode), failing with [`DnfError::NotWellTyped`]
/// otherwise.
///
/// A policy whose condition can never be true (`when { false }`) splits into
/// no policies at all. `max_split_nodes` and `max_cubes` are the
/// [`split_atoms`] and [`Dnf::of`] budgets; exceeding either fails with
/// [`DnfError::TooLarge`].
pub fn split_policy(
    policy: &Policy,
    schema: &Schema,
    max_split_nodes: usize,
    max_cubes: usize,
) -> Result<Vec<Policy>, DnfError> {
    validate(std::iter::once(policy), schema)?;
    split_policy_unchecked(policy, max_split_nodes, max_cubes)
}

/// [`split_policy`] for a policy already validated.
fn split_policy_unchecked(
    policy: &Policy,
    max_split_nodes: usize,
    max_cubes: usize,
) -> Result<Vec<Policy>, DnfError> {
    // No `when` clause means `true` (as `Template::condition` reads it).
    let no_when = bool_lit(true);
    let condition = policy.non_scope_constraints().unwrap_or(&no_when);
    let split = normalize_unchecked(condition, max_split_nodes)?;
    // `can_error` only decides which never-true cubes are kept, and no
    // never-true cube is kept here: erroring policies do not affect the
    // decision.
    let dnf = Dnf::of(&split, |_| false, max_cubes)?;
    dnf.true_cubes()
        .enumerate()
        .map(|(i, cube)| {
            static_instance(
                policy,
                PolicyID::from_string(format!("{}.cube{i}", policy.id())),
                policy.effect(),
                Some(cube.to_expr()),
            )
        })
        .collect()
}

/// Validates `policies` against `schema` in strict mode, as the rewrites of
/// the normalization pipeline assume — each as the *static* policy it splits
/// into (a template-linked policy with its slots filled), not as its
/// template, which the validator would type against every environment the
/// template could be linked in.
pub(super) fn validate<'a>(
    policies: impl Iterator<Item = &'a Policy>,
    schema: &Schema,
) -> Result<(), DnfError> {
    let mut set = PolicySet::new();
    for policy in policies {
        let copy = if policy.is_static() {
            policy.clone()
        } else {
            static_instance(
                policy,
                policy.id().clone(),
                policy.effect(),
                policy.non_scope_constraints().cloned(),
            )?
        };
        set.add(copy)
            .map_err(|_| DnfError::Unsupported("duplicate policy id"))?;
    }
    let result = Validator::new(schema.as_ref().clone()).validate(&set, ValidationMode::Strict);
    if result.validation_passed() {
        Ok(())
    } else {
        Err(DnfError::NotWellTyped {
            errs: result.validation_errors().cloned().collect(),
        })
    }
}

/// A static policy with `policy`'s scope (slots filled), annotations and
/// source location, and the given id, effect and condition: the shape of a
/// cube policy, a combined permit, and a linked policy's static copy.
pub(super) fn static_instance(
    policy: &Policy,
    id: PolicyID,
    effect: Effect,
    condition: Option<Expr>,
) -> Result<Policy, DnfError> {
    StaticPolicy::new(
        id,
        policy.loc().cloned(),
        policy.annotations_arc().as_ref().clone(),
        effect,
        policy.principal_constraint(),
        policy.action_constraint().clone(),
        policy.resource_constraint(),
        condition,
    )
    // Unreachable: a linked policy's scope constraints have their slots
    // filled, and no condition built here contains a slot.
    .map(Into::into)
    .map_err(|_| DnfError::Unsupported("policy with an unfilled slot"))
}

/// [`split_policy`] applied to every policy of the set: the returned set
/// contains one static policy per cube of each input policy and produces the
/// same authorization decision as `policies` on every input. Unlinked
/// templates are not carried over (they do not affect authorization, and are
/// not validated). The budgets apply to each policy afresh, so a set of `n` policies may grow to
/// `n * max_cubes` policies.
pub fn split_policy_set(
    policies: &PolicySet,
    schema: &Schema,
    max_split_nodes: usize,
    max_cubes: usize,
) -> Result<PolicySet, DnfError> {
    validate(policies.policies(), schema)?;
    let mut out = PolicySet::new();
    for policy in policies.policies() {
        for split in split_policy_unchecked(policy, max_split_nodes, max_cubes)? {
            // Unreachable: `id ↦ id + ".cube" + i` is injective (the trailing
            // `".cube" + <digits>` decomposition is unique, as `<digits>`
            // contains no dot) and input ids are unique within the set.
            out.add(split)
                .map_err(|_| DnfError::Unsupported("duplicate split policy id"))?;
        }
    }
    Ok(out)
}
