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

//! Assumptions about concrete (or partially known) entity data and requests.
//!
//! These are *open-world*: an assumption about an entity constrains the
//! symbolic store only at that entity, exactly as Lean's `SameEntities`
//! relates a concrete store to a symbolic one
//! (`cedar-lean/Cedar/Thm/SymCC/Data/Basic.lean`). Entities that are not
//! mentioned stay fully symbolic.

use std::collections::BTreeSet;

use cedar_policy::{RequestEnv, Schema};
use cedar_policy_core::ast::{EntityUID, Value};
use cedar_policy_core::tpe::entities::{PartialEntities, PartialEntity};
use cedar_policy_core::tpe::request::PartialRequest;
use cedar_policy_core::validator::types::{OpenTag, Type};
use cedar_policy_core::validator::ValidatorEntityTypeKind;

use super::err::{EvaluationError, Result};
use crate::symcc::env::{Environment, SymEntityData};
use crate::symcc::term::Term;
use crate::symcc::term_type::TermType;
use crate::symcc::type_abbrevs::{core_entity_type_into_entity_type, core_uid_into_uid};
use crate::term_factory as factory;
use crate::SymEnv;

/// Terms of type `Bool`, plus the footprint terms they mention.
pub(super) struct Asserted {
    pub asserts: Vec<Term>,
    pub footprint: BTreeSet<Term>,
}

impl Asserted {
    fn new() -> Self {
        Self {
            asserts: Vec::new(),
            footprint: BTreeSet::new(),
        }
    }

    fn uid_term(&mut self, uid: &EntityUID) -> Term {
        let term = Term::from(core_uid_into_uid(uid).clone());
        self.footprint.insert(factory::some_of(term.clone()));
        term
    }
}

fn invalid_entities(msg: String) -> EvaluationError {
    EvaluationError::InvalidEntities(msg)
}

/// Asserts that the symbolic store agrees with `entities` on every entity in
/// it: the attributes, the ancestors of each declared ancestor type, and the
/// tags — each only where the partial entity provides them.
///
/// Precondition: `entities` is validated against the schema
/// (`PartialEntities::from_concrete` / `from_entities` do that), because the
/// symbolizer produces ill-typed terms for attribute records that violate it.
pub(super) fn entities_asserts(
    entities: &PartialEntities,
    sym_env: &SymEnv,
    schema: &Schema,
) -> Result<Asserted> {
    let mut out = Asserted::new();
    for entity in entities.entities() {
        entity_asserts(entity, sym_env, schema, &mut out)?;
    }
    Ok(out)
}

fn entity_asserts(
    entity: &PartialEntity,
    sym_env: &SymEnv,
    schema: &Schema,
    out: &mut Asserted,
) -> Result<()> {
    let uid = entity.uid();
    let ety = core_entity_type_into_entity_type(uid.entity_type());
    if !sym_env.entities.is_valid_entity_uid(core_uid_into_uid(uid)) {
        return Err(invalid_entities(format!(
            "entity `{uid}` is not valid for the schema (unknown type, or not a member of an enumerated type)"
        )));
    }
    let Some(data): Option<&SymEntityData> = sym_env.entities.get(ety) else {
        return Err(invalid_entities(format!(
            "entity type `{ety}` not in the schema"
        )));
    };
    let uid_term = out.uid_term(uid);

    // Entity types with declared attributes: standard entity types only.
    // Action types and enumerated types have no attributes (and, for
    // actions, no `ValidatorEntityType` to derive a record type from).
    let validator_ety = schema
        .as_ref()
        .get_entity_type(uid.entity_type())
        .filter(|vty| matches!(vty.kind, ValidatorEntityTypeKind::Standard(_)));

    if let (Some(attrs), Some(vty)) = (entity.attrs(), validator_ety) {
        let record_ty =
            Type::record_with_attributes(vty.attributes().clone(), OpenTag::ClosedAttributes);
        let value = Value::record_arc(std::sync::Arc::new(attrs.clone()), None);
        let literal = Term::from_value(&value, &record_ty)
            .map_err(|e| invalid_entities(format!("attributes of `{uid}`: {e}")))?;
        out.asserts.push(factory::eq(
            factory::app(data.attrs.clone(), uid_term.clone()),
            literal,
        ));
    }

    if let Some(ancestors) = entity.ancestors() {
        for (anc_ty, uf) in &data.ancestors {
            let anc_term_ty = TermType::Entity {
                ety: anc_ty.clone(),
            };
            let ancs = ancestors
                .iter()
                .filter(|anc| core_entity_type_into_entity_type(anc.entity_type()) == anc_ty)
                .map(|anc| out.uid_term(anc))
                .collect::<Vec<_>>();
            out.asserts.push(factory::eq(
                factory::app(uf.clone(), uid_term.clone()),
                factory::set_of(ancs, anc_term_ty),
            ));
        }
    }

    if let (Some(tags), Some(sym_tags), Some(vty)) = (entity.tags(), &data.tags, validator_ety) {
        let Some(tag_ty) = vty.tag_type() else {
            return Err(invalid_entities(format!(
                "entity `{uid}` has tags but its type declares none"
            )));
        };
        out.asserts.push(factory::eq(
            factory::app(sym_tags.keys.clone(), uid_term.clone()),
            factory::set_of(tags.keys().map(|k| Term::from(k.clone())), TermType::String),
        ));
        for (key, value) in tags {
            let literal = Term::from_value(value, tag_ty)
                .map_err(|e| invalid_entities(format!("tag `{key}` of `{uid}`: {e}")))?;
            out.asserts.push(factory::eq(
                factory::app(
                    sym_tags.vals.clone(),
                    factory::tag_of(uid_term.clone(), Term::from(key.clone())),
                ),
                literal,
            ));
        }
    }
    Ok(())
}

/// Asserts that the symbolic request agrees with `request` on the parts it
/// provides. A request whose principal type, action or resource type differ
/// from `req_env`'s cannot occur in `req_env` at all, so the assumption is
/// simply `false` there (and evaluation reports
/// [`EvaluationError::UnsatisfiableAssumptions`]).
pub(super) fn request_asserts(
    request: &PartialRequest,
    req_env: &RequestEnv,
    sym_env: &SymEnv,
    schema: &Schema,
) -> Result<Asserted> {
    let mut out = Asserted::new();
    let same_env = request.principal_type() == req_env.principal().as_ref()
        && request.action() == req_env.action().as_ref()
        && request.resource_type() == req_env.resource().as_ref();
    if !same_env {
        out.asserts.push(false.into());
        return Ok(out);
    }
    if let Some(eid) = &request.principal().eid {
        let uid = EntityUID::from_components(request.principal_type().clone(), eid.clone(), None);
        let term = out.uid_term(&uid);
        out.asserts
            .push(factory::eq(sym_env.request.principal.clone(), term));
    }
    if let Some(eid) = &request.resource().eid {
        let uid = EntityUID::from_components(request.resource_type().clone(), eid.clone(), None);
        let term = out.uid_term(&uid);
        out.asserts
            .push(factory::eq(sym_env.request.resource.clone(), term));
    }
    if let Some(context) = request.context_attrs() {
        let env = Environment::from_request_env(req_env, schema.as_ref())
            .ok_or_else(|| EvaluationError::RequestEnvNotFound(req_env.clone()))?;
        let value = Value::record_arc(context.clone(), None);
        let literal = Term::from_value(&value, &env.context_type())
            .map_err(|e| EvaluationError::InvalidRequest(format!("context: {e}")))?;
        out.asserts
            .push(factory::eq(sym_env.request.context.clone(), literal));
    }
    Ok(out)
}
