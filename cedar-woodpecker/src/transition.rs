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

//! Transition functions: which permissions, in which source environments,
//! give an implicit permission in a target environment, under a condition
//! over the principal, the target request and the intermediate requests.
//!
//! The file format (JSON):
//!
//! ```json
//! { "transitions": [ {
//!     "name": "pod-create-reads-secrets",
//!     "principal": ["User"],
//!     "sources": [ { "action": "Action::\"create\"", "resource": "Pod" } ],
//!     "target":  { "action": "Action::\"get\"", "resource": "Secret" },
//!     "when": "context.resource1.namespace == resource.namespace"
//! } ] }
//! ```
//!
//! `principal` is optional: without it, every principal type applicable to
//! the target action and to every source action. `when` is a Cedar
//! expression over `principal`, `action`, `resource` and `context` (the
//! target request) and `context.action{i}` / `context.resource{i}` /
//! `context.context{i}` (the `i`-th source request, 1-based; its context has
//! the type the schema gives the source action, an empty record when it
//! declares none).

use std::str::FromStr;

use cedar_policy::{EntityTypeName, EntityUid, Expression, RequestEnv, Schema};
use cedar_policy_core::ast::Expr;
use serde::Deserialize;
use smol_str::SmolStr;

use crate::Error;

/// A request environment of a transition: an action and a resource type
/// (the principal type is the transition's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEnv {
    /// The action.
    pub action: EntityUid,
    /// The resource type.
    pub resource: EntityTypeName,
}

impl SourceEnv {
    /// The request environment for principal type `principal`.
    pub fn env(&self, principal: &EntityTypeName) -> RequestEnv {
        RequestEnv::new(
            principal.clone(),
            self.action.clone(),
            self.resource.clone(),
        )
    }
}

/// The context attributes of the extended schema that hold one intermediate
/// request: `action{i}`, `resource{i}`, `context{i}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAttrs {
    /// The action.
    pub action: SmolStr,
    /// The resource.
    pub resource: SmolStr,
    /// The context.
    pub context: SmolStr,
}

/// A transition function `(pt, [(at_i, rt_i)], at', rt', T)`.
#[derive(Debug, Clone)]
pub struct Transition {
    /// The name, for provenance.
    pub name: String,
    /// The principal types, or every applicable one.
    pub principal: Option<Vec<EntityTypeName>>,
    /// The source environments, in order (`context.action1`, … refer to them).
    pub sources: Vec<SourceEnv>,
    /// The target environment.
    pub target: SourceEnv,
    /// The condition `T`.
    pub when: Expr,
}

#[derive(Deserialize)]
struct TransitionsFile {
    transitions: Vec<TransitionSpec>,
}

#[derive(Deserialize)]
struct TransitionSpec {
    name: String,
    #[serde(default)]
    principal: Option<Vec<String>>,
    sources: Vec<EnvSpec>,
    target: EnvSpec,
    #[serde(default = "default_when")]
    when: String,
}

fn default_when() -> String {
    "true".to_string()
}

#[derive(Deserialize)]
struct EnvSpec {
    action: String,
    resource: String,
}

impl EnvSpec {
    fn parse(&self, what: &str) -> Result<SourceEnv, Error> {
        let action = EntityUid::from_str(&self.action)
            .map_err(|e| Error::Transitions(format!("{what}: action `{}`: {e}", self.action)))?;
        let resource = EntityTypeName::from_str(&self.resource).map_err(|e| {
            Error::Transitions(format!("{what}: resource `{}`: {e}", self.resource))
        })?;
        Ok(SourceEnv { action, resource })
    }
}

impl Transition {
    /// Parses a transition file.
    pub fn parse_json(text: &str) -> Result<Vec<Transition>, Error> {
        let file: TransitionsFile =
            serde_json::from_str(text).map_err(|e| Error::Transitions(e.to_string()))?;
        file.transitions
            .iter()
            .map(|spec| {
                let what = format!("transition `{}`", spec.name);
                if spec.sources.is_empty() {
                    return Err(Error::Transitions(format!("{what}: no sources")));
                }
                let principal = spec
                    .principal
                    .as_ref()
                    .map(|names| {
                        names
                            .iter()
                            .map(|n| {
                                EntityTypeName::from_str(n).map_err(|e| {
                                    Error::Transitions(format!("{what}: principal `{n}`: {e}"))
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?;
                let sources = spec
                    .sources
                    .iter()
                    .enumerate()
                    .map(|(i, s)| s.parse(&format!("{what}, source {}", i + 1)))
                    .collect::<Result<Vec<_>, _>>()?;
                let target = spec.target.parse(&format!("{what}, target"))?;
                let when = Expression::from_str(&spec.when)
                    .map_err(|e| Error::Transitions(format!("{what}: when: {e}")))?
                    .as_ref()
                    .clone();
                Ok(Transition {
                    name: spec.name.clone(),
                    principal,
                    sources,
                    target,
                    when,
                })
            })
            .collect()
    }

    /// The principal types the transition applies to: the given ones, or
    /// every type that can be the principal of the target action and of every
    /// source action, by name.
    pub fn principal_types(&self, schema: &Schema) -> Vec<EntityTypeName> {
        if let Some(given) = &self.principal {
            return given.clone();
        }
        let Some(candidates) = schema.principals_for_action(&self.target.action) else {
            return Vec::new();
        };
        let mut types: Vec<EntityTypeName> = candidates
            .filter(|pt| {
                self.sources.iter().all(|s| {
                    schema
                        .principals_for_action(&s.action)
                        .is_some_and(|mut ps| ps.any(|p| p == *pt))
                })
            })
            .cloned()
            .collect();
        types.sort_by_key(ToString::to_string);
        types
    }

    /// The context attribute names the extended schema adds for the `i`-th
    /// source (1-based): `action{i}`, `resource{i}`, `context{i}`.
    pub fn context_attrs(&self) -> Vec<SourceAttrs> {
        (1..=self.sources.len())
            .map(|i| SourceAttrs {
                action: format!("action{i}").into(),
                resource: format!("resource{i}").into(),
                context: format!("context{i}").into(),
            })
            .collect()
    }

    /// Every attribute name the extended schema adds.
    pub fn added_attrs(&self) -> Vec<SmolStr> {
        self.context_attrs()
            .into_iter()
            .flat_map(|a| [a.action, a.resource, a.context])
            .collect()
    }
}
