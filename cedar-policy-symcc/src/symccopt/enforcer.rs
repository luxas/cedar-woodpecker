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

//! This module is as straightforward a translation as possible of
//! <https://github.com/cedar-policy/cedar-spec/blob/main/cedar-lean/Cedar/SymCCOpt/Enforcer.lean>.

use std::collections::BTreeSet;

use crate::symcc::{self, env::SymEntities, term::Term};

use super::{CompiledPolicy, CompiledPolicySet};

pub fn enforce_compiled_policy(cp: &CompiledPolicy) -> BTreeSet<Term> {
    let tr = cp.footprint.iter().flat_map(|term1| {
        cp.footprint
            .iter()
            .map(|term2| symcc::enforcer::transitivity(term1, term2, &cp.symenv.entities))
    });
    cp.acyclicity.iter().cloned().chain(tr).collect()
}

#[expect(dead_code, reason = "exists in the Lean")]
pub fn enforce_compiled_policyset(cpset: &CompiledPolicySet) -> BTreeSet<Term> {
    let tr = cpset.footprint.iter().flat_map(|term1| {
        cpset
            .footprint
            .iter()
            .map(|term2| symcc::enforcer::transitivity(term1, term2, &cpset.symenv.entities))
    });
    cpset.acyclicity.iter().cloned().chain(tr).collect()
}

pub fn enforce_pair_compiled_policy(cp1: &CompiledPolicy, cp2: &CompiledPolicy) -> BTreeSet<Term> {
    assert_eq!(&cp1.symenv, &cp2.symenv);
    let footprint = cp1.footprint.iter().chain(cp2.footprint.iter()); // since `footprint` is just an iterator, it is cheap to clone
    let tr = footprint.clone().flat_map(|term1| {
        footprint
            .clone()
            .map(|term2| symcc::enforcer::transitivity(term1, term2, &cp1.symenv.entities))
    });
    cp1.acyclicity
        .iter()
        .cloned()
        .chain(cp2.acyclicity.iter().cloned())
        .chain(tr)
        .collect()
}

pub fn enforce_pair_compiled_policyset(
    cpset1: &CompiledPolicySet,
    cpset2: &CompiledPolicySet,
) -> BTreeSet<Term> {
    assert_eq!(&cpset1.symenv, &cpset2.symenv);
    let footprint = cpset1.footprint.iter().chain(cpset2.footprint.iter()); // since `footprint` is just an iterator, it is cheap to clone
    let tr = footprint.clone().flat_map(|term1| {
        footprint
            .clone()
            .map(|term2| symcc::enforcer::transitivity(term1, term2, &cpset1.symenv.entities))
    });
    cpset1
        .acyclicity
        .iter()
        .cloned()
        .chain(cpset2.acyclicity.iter().cloned())
        .chain(tr)
        .collect()
}

/// Returns the acyclicity and transitivity constraints for an arbitrary
/// footprint (a set of option-entity-typed terms), i.e. the n-ary sibling of
/// `enforce_compiled_policy()` / `enforce_pair_compiled_policy()`.
///
/// Callers that combine terms from several compiled expressions in one query
/// must pass the *union* of their footprints here rather than concatenating
/// per-expression results: the transitivity constraints range over all pairs,
/// including cross-expression pairs.
///
/// Not present in the Lean. Used by the symbolic evaluator.
pub(crate) fn enforce_footprint(footprint: &BTreeSet<Term>, es: &SymEntities) -> BTreeSet<Term> {
    let ac = footprint.iter().map(|t| symcc::enforcer::acyclicity(t, es));
    let tr = footprint.iter().flat_map(|t1| {
        footprint
            .iter()
            .map(|t2| symcc::enforcer::transitivity(t1, t2, es))
    });
    ac.chain(tr).collect()
}
