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

//! Identity assumption by lowering (plan 18, Layer B).
//!
//! An *impersonation* says that a principal, by holding some source
//! permission(s), comes to act **as** another identity (a principal type,
//! `assume`) — and so gains everything that identity can do. This is not one
//! `cube ⟹ cube` transition; it is a family, one per `(action, resource)` the
//! assumed identity holds. [`Impersonation::lower`] expands it, against the
//! source cubes already computed from the policy set, into ordinary
//! [`Transition`]s that the existing [`crate::escalate`] handles unchanged:
//! for every environment the assumed identity has a source cube in, a
//! transition whose sources are the impersonation's own sources plus that
//! environment carried by the assumed principal, whose target is the acting
//! principal in that environment, and whose `when` is the impersonation's
//! `when` (with `context.assumed` resolved to the assumed source's
//! `context.principal{k}`) conjoined with `context.resource{k} == resource`.
//!
//! Soundness is inherited, not added: the assumed identity's permission is a
//! real source cube, so a lowered transition only yields a path when that
//! identity actually holds the permission and the binding holds; the per-path
//! solver check in `escalate` is unchanged. Pruning to environments that have
//! a cube keeps the family to the identity's actual permissions rather than
//! the whole schema.
//!
//! File format (JSON):
//! ```json
//! { "impersonations": [ {
//!     "name": "create-pod-assumes-account",
//!     "principal": ["User"],
//!     "sources": [ { "action": "Action::\"create\"", "resource": "Pod" } ],
//!     "assume": "Account",
//!     "when": "context.assumed.name == context.resource1.runsAs"
//! } ] }
//! ```
//! `assume` is a principal type; `when` refers to the assumed identity as
//! `context.assumed` (rewritten to the real `context.principal{k}` on
//! lowering). `principal` (the acting type) and `when` are optional.

use std::collections::BTreeSet;
use std::str::FromStr;

use cedar_policy::{EntityTypeName, EntityUid, Expression};
use cedar_policy_core::ast::{Expr, Var};
use serde::Deserialize;

use crate::cubes::SourceCube;
use crate::expr::{and_chain, context_attr, replace};
use crate::transition::{SourceEnv, Transition};
use crate::Error;

/// An impersonation: acting principal(s) that, holding `sources`, assume the
/// identity `assume`, bound by `when`.
#[derive(Debug, Clone)]
pub struct Impersonation {
    /// The name, for provenance (each lowered transition is `name@action:resource`).
    pub name: String,
    /// The acting principal types, or every applicable one.
    pub principal: Option<Vec<EntityTypeName>>,
    /// The acting principal's own source environments (default-identity).
    pub sources: Vec<SourceEnv>,
    /// The assumed identity's principal type.
    pub assume: EntityTypeName,
    /// The binding condition; refers to the assumed identity as `context.assumed`.
    pub when: Expr,
}

#[derive(Deserialize)]
struct ImpersonationsFile {
    impersonations: Vec<ImpersonationSpec>,
}

#[derive(Deserialize)]
struct ImpersonationSpec {
    name: String,
    #[serde(default)]
    principal: Option<Vec<String>>,
    sources: Vec<EnvSpec>,
    assume: String,
    #[serde(default = "default_when")]
    when: String,
}

#[derive(Deserialize)]
struct EnvSpec {
    #[serde(default)]
    principal: Option<String>,
    action: String,
    resource: String,
}

fn default_when() -> String {
    "true".to_string()
}

impl EnvSpec {
    fn parse(&self, what: &str) -> Result<SourceEnv, Error> {
        let principal = self
            .principal
            .as_ref()
            .map(|p| {
                EntityTypeName::from_str(p)
                    .map_err(|e| Error::Transitions(format!("{what}: principal `{p}`: {e}")))
            })
            .transpose()?;
        let action = EntityUid::from_str(&self.action)
            .map_err(|e| Error::Transitions(format!("{what}: action `{}`: {e}", self.action)))?;
        let resource = EntityTypeName::from_str(&self.resource).map_err(|e| {
            Error::Transitions(format!("{what}: resource `{}`: {e}", self.resource))
        })?;
        Ok(SourceEnv {
            principal,
            action,
            resource,
        })
    }
}

impl Impersonation {
    /// Parses an impersonation file.
    pub fn parse_json(text: &str) -> Result<Vec<Impersonation>, Error> {
        let file: ImpersonationsFile =
            serde_json::from_str(text).map_err(|e| Error::Transitions(e.to_string()))?;
        file.impersonations
            .iter()
            .map(|spec| {
                let what = format!("impersonation `{}`", spec.name);
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
                let assume = EntityTypeName::from_str(&spec.assume).map_err(|e| {
                    Error::Transitions(format!("{what}: assume `{}`: {e}", spec.assume))
                })?;
                let when = Expression::from_str(&spec.when)
                    .map_err(|e| Error::Transitions(format!("{what}: when: {e}")))?
                    .as_ref()
                    .clone();
                Ok(Impersonation {
                    name: spec.name.clone(),
                    principal,
                    sources,
                    assume,
                    when,
                })
            })
            .collect()
    }

    /// Lowers this impersonation to ordinary transitions, one per distinct
    /// environment the assumed identity has a source cube in (`cubes`). Each
    /// lowered transition appends the assumed environment, carried by the
    /// assumed principal, as its last source (`k = sources.len() + 1`); its
    /// `when` is this impersonation's `when` with `context.assumed` resolved
    /// to `context.principal{k}`, conjoined with `context.resource{k} ==
    /// resource` so the assumed permission is on the target's own resource.
    pub fn lower(&self, cubes: &[SourceCube]) -> Vec<Transition> {
        let k = self.sources.len() + 1;
        let principal_k = context_attr(&format!("principal{k}"));
        let resource_k = context_attr(&format!("resource{k}"));
        let bound_when = replace(&self.when, &context_attr("assumed"), &principal_k);
        let when = and_chain([
            bound_when,
            Expr::is_eq(resource_k, Expr::var(Var::Resource)),
        ])
        .unwrap_or_else(|| Expr::val(true));

        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut out = Vec::new();
        for cube in cubes {
            if cube.env.principal() != &self.assume {
                continue;
            }
            let action = cube.env.action().clone();
            let resource = cube.env.resource().clone();
            if !seen.insert((action.to_string(), resource.to_string())) {
                continue;
            }
            let mut sources = self.sources.clone();
            sources.push(SourceEnv {
                principal: Some(self.assume.clone()),
                action: action.clone(),
                resource: resource.clone(),
            });
            out.push(Transition {
                name: format!("{}@{}:{}", self.name, action.id().unescaped(), resource.basename()),
                principal: self.principal.clone(),
                sources,
                target: SourceEnv {
                    principal: None,
                    action,
                    resource,
                },
                when: when.clone(),
            });
        }
        out
    }
}
