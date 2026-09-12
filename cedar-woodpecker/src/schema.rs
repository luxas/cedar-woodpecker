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

//! The extended schema of a transition: a copy of the schema in which the
//! target action's context carries the intermediate requests — required
//! attributes `action{i}` (of the `i`-th source action's type),
//! `resource{i}` (of the `i`-th source resource type) and `context{i}` (of
//! the `i`-th source action's context type; an empty record when it declares
//! none).
//!
//! Each of the three is typed through a fresh common type
//! (`__woodpecker_action{i}`, `__woodpecker_resource{i}`,
//! `__woodpecker_context{i}`) added to the namespace that *defines* the
//! thing it names — the source action's, the resource type's — and referenced
//! from the target action's context by its qualified name (bare for the root
//! namespace). Cedar then resolves every name inside the alias where it was
//! written: a bare `Action` in the source action's namespace, a context
//! record's unqualified entity and common types in theirs, an alias of an
//! alias through its own namespace. The target action's own context, to
//! which the attributes are added, has to become an inline record; when the
//! record comes from another namespace, its unqualified names are qualified
//! with that namespace where it defines them (plan 16). Made on the JSON
//! schema form, which is the same whatever syntax the schema came in.

use cedar_policy::{EntityUid, Schema, SchemaFragment};
use serde_json::{json, Map, Value};
use smol_str::SmolStr;

use crate::transition::Transition;
use crate::Error;

/// A schema extended for one transition.
#[derive(Debug, Clone)]
pub struct ExtendedSchema {
    /// The extended schema.
    pub schema: Schema,
    /// The same as a fragment (to print it: `to_cedarschema`).
    pub fragment: SchemaFragment,
    /// The same as JSON.
    pub json: Value,
    /// The attribute names added to the target action's context
    /// ([`Transition::added_attrs`]).
    pub added: Vec<SmolStr>,
}

fn bad(what: &str) -> Error {
    Error::Schema(format!("cannot extend the schema: {what}"))
}

/// The namespace object `ns` of the JSON schema `json` (`""` for the root).
fn namespace<'a>(json: &'a Value, ns: &str) -> Result<&'a Map<String, Value>, Error> {
    json.get(ns)
        .and_then(Value::as_object)
        .ok_or_else(|| bad(&format!("no namespace `{ns}`")))
}

/// The `appliesTo` object of `action`.
fn applies_to<'a>(json: &'a Value, action: &EntityUid) -> Result<&'a Map<String, Value>, Error> {
    let ns = action.type_name().namespace();
    let id = action.id().unescaped().to_string();
    namespace(json, &ns)?
        .get("actions")
        .and_then(|a| a.get(&id))
        .ok_or_else(|| bad(&format!("no action `{id}` in namespace `{ns}`")))?
        .get("appliesTo")
        .and_then(Value::as_object)
        .ok_or_else(|| bad(&format!("action `{id}` applies to nothing")))
}

/// A reference to the common type `name` of namespace `ns`.
fn reference(ns: &str, name: &str) -> Value {
    let qualified = if ns.is_empty() {
        name.to_string()
    } else {
        format!("{ns}::{name}")
    };
    json!({ "type": "EntityOrCommon", "name": qualified })
}

/// Adds the common type `name` = `ty` to namespace `ns` of `json`.
fn add_common_type(json: &mut Value, ns: &str, name: &str, ty: Value) -> Result<(), Error> {
    let common = json
        .get_mut(ns)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| bad(&format!("no namespace `{ns}`")))?
        .entry("commonTypes")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            bad(&format!(
                "`commonTypes` of namespace `{ns}` is not an object"
            ))
        })?;
    if common.contains_key(name) {
        return Err(bad(&format!(
            "namespace `{ns}` already has a common type `{name}`"
        )));
    }
    common.insert(name.to_string(), ty);
    Ok(())
}

/// Whether `ns` defines `name` as an entity or a common type.
fn defines(json: &Value, ns: &str, name: &str) -> bool {
    namespace(json, ns).is_ok_and(|n| {
        ["entityTypes", "commonTypes"]
            .iter()
            .any(|k| n.get(*k).and_then(|m| m.get(name)).is_some())
    })
}

/// The name a type value refers to (`{"type": "Name"}` or `{"type":
/// "EntityOrCommon", "name": "Name"}`), if it is not an inline type.
fn type_name(ty: &Value) -> Result<Option<&str>, Error> {
    let kind = ty
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("a type has no `type`"))?;
    Ok(match kind {
        "Record" | "Set" | "Entity" | "Extension" | "String" | "Long" | "Boolean" | "Bool" => None,
        "EntityOrCommon" => Some(
            ty.get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("an `EntityOrCommon` type has no `name`"))?,
        ),
        other => Some(other),
    })
}

/// `ty`, written in namespace `ns`, as an inline `Record` type together
/// with the namespace it was found in: itself, or the common type it names
/// — looked up in its own namespace when qualified, in `ns` and then in the
/// root when not, and followed through an alias of an alias. Annotations
/// are dropped (they do not belong to an attribute type).
fn resolve_record(json: &Value, ns: &str, ty: &Value) -> Result<(String, Value), Error> {
    let Some(name) = type_name(ty)? else {
        if ty.get("type").and_then(Value::as_str) != Some("Record") {
            return Err(bad("the target action's context is not a record"));
        }
        let keep: Map<String, Value> = ty
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| k.as_str() == "type" || k.as_str() == "attributes")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Ok((ns.to_string(), Value::Object(keep)));
    };
    let (found_ns, local) = match name.rsplit_once("::") {
        Some((qualifier, local)) => (qualifier.to_string(), local),
        None if defines(json, ns, name) => (ns.to_string(), name),
        None => (String::new(), name),
    };
    let common = namespace(json, &found_ns)?
        .get("commonTypes")
        .and_then(|c| c.get(local))
        .ok_or_else(|| bad(&format!("context type `{name}` is not a record")))?;
    resolve_record(json, &found_ns, common)
}

/// Qualifies, in place, the unqualified type names inside `ty` (written in
/// namespace `ns`) that `ns` defines, so that the type means the same when
/// inlined elsewhere.
fn qualify(ty: &mut Value, json: &Value, ns: &str) {
    if ns.is_empty() {
        return;
    }
    let Some(obj) = ty.as_object_mut() else {
        return;
    };
    let kind = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match kind.as_str() {
        "Record" => {
            if let Some(attrs) = obj.get_mut("attributes").and_then(Value::as_object_mut) {
                for v in attrs.values_mut() {
                    qualify(v, json, ns);
                }
            }
        }
        "Set" => {
            if let Some(elem) = obj.get_mut("element") {
                qualify(elem, json, ns);
            }
        }
        "Entity" | "EntityOrCommon" => {
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !name.contains("::") && defines(json, ns, &name) {
                obj.insert("name".to_string(), json!(format!("{ns}::{name}")));
            }
        }
        "Extension" | "String" | "Long" | "Boolean" | "Bool" => {}
        name => {
            if !name.contains("::") && defines(json, ns, name) {
                obj.insert("type".to_string(), json!("EntityOrCommon"));
                obj.insert("name".to_string(), json!(format!("{ns}::{name}")));
            }
        }
    }
}

/// The extended schema of `transition`, from the JSON form `base` of the
/// schema.
pub fn extend_schema(base: &Value, transition: &Transition) -> Result<ExtendedSchema, Error> {
    let target = &transition.target.action;
    let target_ns = target.type_name().namespace();
    // the aliases, from the unedited schema
    let mut aliases: Vec<(String, String, Value)> = Vec::new();
    let mut refs: Vec<(SmolStr, Value)> = Vec::new();
    for (attrs, source) in transition
        .context_attrs()
        .into_iter()
        .zip(&transition.sources)
    {
        let action_ns = source.action.type_name().namespace();
        let resource_ns = source.resource.namespace();
        let context = applies_to(base, &source.action)?
            .get("context")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "Record", "attributes": {} }));
        for (attr, ns, ty) in [
            (
                &attrs.action,
                action_ns.clone(),
                json!({ "type": "Entity", "name": "Action" }),
            ),
            (
                &attrs.resource,
                resource_ns,
                json!({ "type": "Entity", "name": source.resource.basename() }),
            ),
            (&attrs.context, action_ns, context),
        ] {
            let alias = format!("__woodpecker_{attr}");
            refs.push((attr.clone(), reference(&ns, &alias)));
            aliases.push((ns, alias, ty));
        }
    }
    let mut json = base.clone();
    for (ns, name, ty) in aliases {
        add_common_type(&mut json, &ns, &name, ty)?;
    }
    // the target action's context as an inline record, the attributes added
    let mut context = match applies_to(base, target)?.get("context") {
        None => json!({ "type": "Record", "attributes": {} }),
        Some(ty) => {
            let (from_ns, mut record) = resolve_record(base, &target_ns, ty)?;
            if from_ns != target_ns {
                qualify(&mut record, base, &from_ns);
            }
            record
        }
    };
    let action_id = target.id().unescaped().to_string();
    let attributes = context
        .get_mut("attributes")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| bad("context record has no attributes"))?;
    for (name, mut ty) in refs {
        if attributes.contains_key(name.as_str()) {
            return Err(bad(&format!(
                "the context of `{action_id}` already has an attribute `{name}`"
            )));
        }
        if let Some(m) = ty.as_object_mut() {
            m.insert("required".to_string(), Value::Bool(true));
        }
        attributes.insert(name.to_string(), ty);
    }
    json.get_mut(target_ns.as_str())
        .and_then(|n| n.get_mut("actions"))
        .and_then(|a| a.get_mut(&action_id))
        .and_then(|a| a.get_mut("appliesTo"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| bad(&format!("action `{action_id}` applies to nothing")))?
        .insert("context".to_string(), context);
    let fragment = SchemaFragment::from_json_value(json.clone())
        .map_err(|e| Error::Schema(format!("the extended schema does not parse: {e}")))?;
    let schema = Schema::from_schema_fragments([fragment.clone()])
        .map_err(|e| Error::Schema(format!("the extended schema does not validate: {e}")))?;
    Ok(ExtendedSchema {
        schema,
        fragment,
        json,
        added: transition.added_attrs(),
    })
}
