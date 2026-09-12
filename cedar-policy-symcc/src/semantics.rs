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

//! Semantic assumptions declared in a schema: `@semantics("<expr>")`
//! annotations on a namespace, an entity type or an action,
//! each a Cedar boolean expression the analyses assume `true` in every
//! request environment.
//!
//! An assumption is *global* wherever it is written — the placement is only
//! organisational — so it must typecheck in every request environment of the
//! schema; the author guards it with `if <var> is <type> then … else true`,
//! and writes an implication `A => B` as `if A then B else true`. Several
//! assumptions on one declaration are conjoined with `&&`.
//!
//! Later iterations: `@semantics("file:…")` referring to a `.cedarsemantics`
//! file; local invariants on common types (`this` substituted per use of the
//! type, guarded by `<var> is <type> && <var> has <path>`).

use std::str::FromStr;

use cedar_policy::{EntityNamespace, Schema, SchemaFragment};
use cedar_policy_core::ast::Expr;

/// The annotation key.
pub const SEMANTICS_ANNOTATION: &str = "semantics";

/// One `@semantics` assumption.
#[derive(Debug, Clone)]
pub struct Assumption {
    /// The declaration the annotation is on, for messages: `namespace k8s`,
    /// `entity type k8s::User`, `action k8s::Action::"get"`.
    pub origin: String,
    /// The expression, assumed `true`.
    pub expression: Expr,
    /// The annotation's value as written.
    pub source: String,
}

/// Why the `@semantics` annotations could not be collected. Every message
/// starts with `the @semantics`, which the VS Code extension relies on.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SemanticsError {
    /// An annotation's value is not a Cedar expression.
    #[error("the @{SEMANTICS_ANNOTATION} annotation on {origin} does not parse: {message}")]
    Parse {
        /// The declaration the annotation is on.
        origin: String,
        /// The parser's message.
        message: String,
    },
    /// The schema text does not parse as a fragment (it was accepted as a
    /// schema, so this should not happen; reported rather than silently
    /// dropping the assumptions).
    #[error("the @{SEMANTICS_ANNOTATION} annotations cannot be read, the schema does not parse as a fragment: {0}")]
    Fragment(String),
    /// A declaration's namespace does not parse as one (same caveat).
    #[error("the @{SEMANTICS_ANNOTATION} annotations cannot be read, the namespace of {0} does not parse")]
    Namespace(String),
}

/// Collects the `@semantics` assumptions of a schema, given its source text
/// (`json` says which syntax) and the schema built from it. The fragment is
/// parsed again from the text, since annotations live on the fragment.
/// Namespaces, entity types and actions are covered; common types are not
/// enumerable through the public API and are left for a later iteration.
pub fn collect_semantics(
    schema_text: &str,
    json: bool,
    schema: &Schema,
) -> Result<Vec<Assumption>, SemanticsError> {
    let fragment = if json {
        SchemaFragment::from_json_str(schema_text).map_err(|e| e.to_string())
    } else {
        SchemaFragment::from_cedarschema_str(schema_text)
            .map(|(f, _warnings)| f)
            .map_err(|e| e.to_string())
    }
    .map_err(SemanticsError::Fragment)?;
    let mut out = Vec::new();
    let mut push = |origin: String, value: &str| -> Result<(), SemanticsError> {
        let expression = Expr::from_str(value).map_err(|e| SemanticsError::Parse {
            origin: origin.clone(),
            message: e.to_string(),
        })?;
        out.push(Assumption {
            origin,
            expression,
            source: value.to_string(),
        });
        Ok(())
    };
    for namespace in fragment.namespaces().flatten() {
        if let Some(value) = fragment.namespace_annotation(namespace.clone(), SEMANTICS_ANNOTATION)
        {
            push(format!("namespace {namespace}"), value)?;
        }
    }
    for ety in schema.entity_types() {
        let namespace = namespace_of(&ety.namespace(), || format!("entity type {ety}"))?;
        if let Some(value) =
            fragment.entity_type_annotation(namespace, ety.basename(), SEMANTICS_ANNOTATION)
        {
            push(format!("entity type {ety}"), value)?;
        }
    }
    for action in schema.actions() {
        let namespace = namespace_of(&action.type_name().namespace(), || {
            format!("action {action}")
        })?;
        if let Some(value) =
            fragment.action_annotation(namespace, action.id(), SEMANTICS_ANNOTATION)
        {
            push(format!("action {action}"), value)?;
        }
    }
    Ok(out)
}

/// The namespace of a declaration (`None` for the empty namespace), or an
/// error naming the declaration — never silently the empty namespace, which
/// would look up the annotation under the wrong name.
fn namespace_of(
    namespace: &str,
    declaration: impl FnOnce() -> String,
) -> Result<Option<EntityNamespace>, SemanticsError> {
    if namespace.is_empty() {
        Ok(None)
    } else {
        EntityNamespace::from_str(namespace)
            .map(Some)
            .map_err(|_| SemanticsError::Namespace(declaration()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"
@semantics("if principal is k8s::User then principal.groups.contains(\"system:authenticated\") else true")
namespace k8s {
  @semantics("true")
  entity User { groups: Set<String> };
  entity Secret { name: String };
  @semantics("if action == k8s::Action::\"get\" then !(resource has name) else true")
  action get appliesTo { principal: User, resource: Secret };
  action create appliesTo { principal: User, resource: Secret };
}
"#;

    #[test]
    fn collects_every_placement() {
        let schema = Schema::from_cedarschema_str(SCHEMA).unwrap().0;
        let found = collect_semantics(SCHEMA, false, &schema).unwrap();
        let mut origins: Vec<String> = found.iter().map(|a| a.origin.clone()).collect();
        origins.sort();
        assert_eq!(
            origins,
            vec![
                r#"action k8s::Action::"get""#.to_string(),
                "entity type k8s::User".to_string(),
                "namespace k8s".to_string(),
            ]
        );
        assert!(found.iter().all(|a| !a.source.is_empty()));
    }

    #[test]
    fn reports_a_bad_expression() {
        let text = r#"
namespace k8s {
  @semantics("if principal is then")
  entity User { groups: Set<String> };
  action get appliesTo { principal: User, resource: User };
}
"#;
        let schema = Schema::from_cedarschema_str(text).unwrap().0;
        let err = collect_semantics(text, false, &schema).unwrap_err();
        assert!(
            matches!(&err, SemanticsError::Parse { origin, .. } if origin == "entity type k8s::User"),
            "{err:?}"
        );
        assert!(err
            .to_string()
            .contains("@semantics annotation on entity type k8s::User"));
    }

    #[test]
    fn none_is_fine() {
        let text = r#"entity User; action get appliesTo { principal: User, resource: User };"#;
        let schema = Schema::from_cedarschema_str(text).unwrap().0;
        assert!(collect_semantics(text, false, &schema).unwrap().is_empty());
    }
}
