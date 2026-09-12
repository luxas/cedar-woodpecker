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

//! The policy simplifier (`cedar_policy_symcc::simplify`): which expressions
//! it pinpoints as statically determined, and which it leaves alone.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "test code"
)]

use cedar_policy::Schema;
use cedar_policy_symcc::evaluator::Evaluator;
use cedar_policy_symcc::semantics::collect_semantics;
use cedar_policy_symcc::simplify::{simplify, Finding, FindingKind, SimplifyError};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;

const SCHEMA: &str = r#"
entity User { a: Bool, b: Bool, age: Long, nick?: String } tags Long;
entity Doc { owner: User, protected: Bool };
action view appliesTo { principal: User, resource: Doc, context: { admin: Bool } };
action edit appliesTo { principal: User, resource: Doc, context: { admin: Bool } };
"#;

fn schema() -> Schema {
    Schema::from_cedarschema_str(SCHEMA).unwrap().0
}

/// The findings of `policies`, as `(source text, kind)`.
async fn findings(policies: &str) -> Vec<(String, FindingKind)> {
    let schema = schema();
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap()).unwrap();
    let evaluator = Evaluator::new(compiler, &schema).unwrap();
    let (found, _) = simplify(evaluator, &schema, policies, &[]).await.unwrap();
    found
        .iter()
        .map(|f: &Finding| (policies[f.offset..f.offset + f.length].to_string(), f.kind))
        .collect()
}

/// The plan's example: `(a && b) || (a && !a)` — the innermost cause is the
/// `a` inside `!a`, always true given the `&&` before it; `!a` and the
/// branch are implied and not reported again.
#[tokio::test]
async fn dead_branch_innermost_cause() {
    let found = findings(
        "permit(principal, action, resource) when {
           (principal.a && principal.b) || (principal.a && !principal.a)
         };",
    )
    .await;
    assert_eq!(
        found,
        vec![("principal.a".to_string(), FindingKind::NeverFalse)]
    );
}

/// Nothing to report on a policy whose atoms can all go both ways.
#[tokio::test]
async fn clean_policy() {
    let found = findings(
        "permit(principal, action, resource) when {
           principal.a && (principal.b || resource.protected)
         };",
    )
    .await;
    assert!(found.is_empty(), "{found:?}");
}

/// A warning needs every request environment to agree: an atom that is true
/// in one environment and false in another is not reported, while the
/// disjunction that is true in both is.
#[tokio::test]
async fn every_environment_must_agree() {
    let found = findings(
        r#"permit(principal, action, resource) when {
           (action == Action::"view" || action == Action::"edit") && context.admin
         };"#,
    )
    .await;
    assert_eq!(
        found,
        vec![(
            r#"action == Action::"view" || action == Action::"edit""#.to_string(),
            FindingKind::NeverFalse
        )]
    );
}

/// An environment the policy cannot apply to (its scope names another
/// action) is dropped from consideration, so the body is judged in the
/// environments that remain: `age + 1 > 18` is true or an overflow error
/// given `age > 18`.
#[tokio::test]
async fn inapplicable_environments_are_dropped() {
    let found = findings(
        r#"permit(principal, action == Action::"view", resource) when {
           principal.age > 18 && principal.age + 1 > 18
         };"#,
    )
    .await;
    assert_eq!(
        found,
        vec![(
            "principal.age + 1 > 18".to_string(),
            FindingKind::NeverFalse
        )]
    );
}

/// A warning speaks for every request environment: the second disjunct
/// runs only in the `edit` environment (in `view` the first disjunct is
/// true and the typechecker folds the rest away), so nothing about it is
/// reported even though `context.admin && context.admin` is redundant
/// there — and the first disjunct, true in one environment and false in the
/// other, is not reported either.
#[tokio::test]
async fn not_evaluated_everywhere_is_not_judged() {
    let found = findings(
        r#"permit(principal, action, resource) when {
           action == Action::"view" || (context.admin && context.admin)
         };"#,
    )
    .await;
    assert!(found.is_empty(), "{found:?}");
}

/// Once a conjunct can never be true in an environment, the policy is false
/// there and later nodes are judged without it: `context.admin` after the
/// `action == "view"` conjunct is redundant in the one environment left.
#[tokio::test]
async fn false_environments_are_dropped_for_later_nodes() {
    let found = findings(
        r#"permit(principal, action, resource) when {
           action == Action::"view" && context.admin && context.admin
         };"#,
    )
    .await;
    assert_eq!(
        found,
        vec![("context.admin".to_string(), FindingKind::NeverFalse)]
    );
}

/// A policy that is false everywhere with no inner cause is reported as a
/// whole (the typechecker folds its condition to a literal, which has no
/// location of its own); an atom that is true unless it errors is reported
/// as never false.
#[tokio::test]
async fn whole_policy_and_errors() {
    let found = findings(
        r#"permit(principal, action, resource) when { false };
           permit(principal, action, resource) when { principal.age + 1 > principal.age };"#,
    )
    .await;
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].1, FindingKind::NeverTrue);
    assert!(found[0].0.starts_with("permit"), "{found:?}");
    assert_eq!(
        found[1],
        (
            "principal.age + 1 > principal.age".to_string(),
            FindingKind::NeverFalse
        )
    );
}

/// Policies that do not validate, and template-linked policies, are skipped.
#[tokio::test]
async fn invalid_and_templates_are_skipped() {
    let found = findings(
        r#"permit(principal, action, resource) when { principal.nope };
           permit(principal == ?principal, action, resource) when { principal.a && !principal.a };"#,
    )
    .await;
    assert!(found.is_empty(), "{found:?}");
}

/// The conjunct that makes a policy false in some environment is judged in
/// every environment, wherever it sits: parenthesised, first of two `when`
/// clauses, or inside a nested `&&` — never reported as "always true" on
/// the strength of the environments where it is true.
#[tokio::test]
async fn false_conjunct_is_judged_wherever_it_sits() {
    for policy in [
        r#"permit(principal, action, resource) when { (action == Action::"view") && context.admin };"#,
        r#"permit(principal, action, resource) when { action == Action::"view" } when { context.admin };"#,
        r#"permit(principal, action, resource) when { (action == Action::"view" && principal.a) && principal.b };"#,
        r#"permit(principal, action, resource) when { (!(action == Action::"edit")) && principal.b };"#,
    ] {
        let found = findings(policy).await;
        assert!(found.is_empty(), "{policy}: {found:?}");
    }
}

/// Literals the user wrote are their own explanation and are not reported;
/// the redundant conjunct after one still is.
#[tokio::test]
async fn literals_are_not_reported() {
    let found = findings(
        "permit(principal, action, resource) when { true && principal.a && principal.a };",
    )
    .await;
    assert_eq!(
        found,
        vec![("principal.a".to_string(), FindingKind::NeverFalse)]
    );
}

/// The plan's kubernetes shape, with the authorizer's convention stated in
/// the schema: reads carry no request body.
const K8S_SCHEMA: &str = r#"
namespace k8s {
  @semantics("if resource is core::secrets && (action == k8s::Action::\"get\" || action == k8s::Action::\"list\") then !(resource has request) else true")
  entity User { groups: Set<String> };
  action get, list, create appliesTo { principal: User, resource: [core::secrets, core::pods] };
}
namespace core {
  entity secrets { request?: { v1: { type: String } } };
  entity pods { name: String };
}
"#;

const K8S_ASSUMPTION: &str = r#"if resource is core::secrets && (action == k8s::Action::\"get\" || action == k8s::Action::\"list\") then !(resource has request) else true"#;

/// The findings of `policies` under `schema_text`'s `@semantics`.
async fn findings_with_semantics(
    schema_text: &str,
    policies: &str,
) -> Result<Vec<(String, FindingKind)>, SimplifyError> {
    let schema = Schema::from_cedarschema_str(schema_text).unwrap().0;
    let assumptions = collect_semantics(schema_text, false, &schema).unwrap();
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap()).unwrap();
    let evaluator = Evaluator::new(compiler, &schema).unwrap();
    let (found, _) = simplify(evaluator, &schema, policies, &assumptions).await?;
    Ok(found
        .iter()
        .map(|f: &Finding| (policies[f.offset..f.offset + f.length].to_string(), f.kind))
        .collect())
}

/// Under the convention, the plan's policy is statically false at its
/// `has` (the whole `has request.v1.type` chain shares one source span);
/// without it, nothing is known.
#[tokio::test]
async fn semantics_make_the_has_false() {
    let policy = r#"permit(
  principal is k8s::User,
  action in [k8s::Action::"get", k8s::Action::"list"],
  resource is core::secrets
) when {
  principal.groups.contains("classified") &&
  resource has request.v1.type &&
  resource.request.v1.type == "kubernetes.io/tls"
};"#;
    let found = findings_with_semantics(K8S_SCHEMA, policy).await.unwrap();
    assert_eq!(
        found,
        vec![(
            "resource has request.v1.type".to_string(),
            FindingKind::NeverTrue
        )]
    );
    let plain = K8S_SCHEMA.replace(&format!("@semantics(\"{K8S_ASSUMPTION}\")"), "");
    assert!(!plain.contains("@semantics"), "{plain}");
    let found = findings_with_semantics(&plain, policy).await.unwrap();
    assert!(found.is_empty(), "{found:?}");
}

/// An assumption that is well typed for pods but not for secrets is an
/// error naming the annotation, the expression and the secrets environment;
/// the `resource is core::pods` guard makes it typecheck everywhere.
#[tokio::test]
async fn unguarded_assumption_is_an_error() {
    let unguarded = r#"resource.name != \"\""#;
    let schema = K8S_SCHEMA.replace(K8S_ASSUMPTION, unguarded);
    let err = findings_with_semantics(&schema, "permit(principal, action, resource);")
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("entity type k8s::User"), "{message}");
    assert!(message.contains("(`resource.name != \"\"`)"), "{message}");
    assert!(message.contains("resource: core::secrets"), "{message}");
    let guarded = format!("if resource is core::pods then {unguarded} else true");
    let schema = K8S_SCHEMA.replace(K8S_ASSUMPTION, &guarded);
    // a policy without a condition is not reported (nothing to simplify)
    let found = findings_with_semantics(&schema, "permit(principal, action, resource);")
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
    let found = findings_with_semantics(
        &schema,
        r#"permit(principal, action, resource is core::pods) when { resource.name == "" };"#,
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        vec![(r#"resource.name == """#.to_string(), FindingKind::NeverTrue)]
    );
}

/// Contradictory assumptions are an error naming the environment, even
/// when no policy gets evaluated (an empty document).
#[tokio::test]
async fn contradictory_assumptions_are_an_error() {
    let schema = K8S_SCHEMA.replace(
        K8S_ASSUMPTION,
        r#"principal.groups.contains(\"x\") && !principal.groups.contains(\"x\")"#,
    );
    let err = findings_with_semantics(&schema, "").await.unwrap_err();
    assert!(
        matches!(err, SimplifyError::UnsatisfiableAssumptions { .. }),
        "{err}"
    );
}

/// The test schema with `@semantics` saying `nick` is always present and the
/// tag `t` always set on a `User`.
fn present_schema() -> String {
    SCHEMA.replace(
        "entity User {",
        r#"@semantics("if principal is User then principal has nick && principal.hasTag(\"t\") else true")
entity User {"#,
    )
}

/// Plan 7: a `has`/`hasTag` guard that the assumptions make always true is
/// not reported when a later access in its scope needs it (strict validation
/// rejects the policy without the guard). (A required attribute's `has`
/// needs no guard and is never kept, but it is not a finding here either:
/// the simplifier assumes no entity data, so it is `false` for a missing
/// principal.)
#[tokio::test]
async fn needed_guards_are_not_reported() {
    let schema = present_schema();
    let policy = |when: &str| format!("permit(principal, action, resource) when {{ {when} }};");
    for when in [
        r#"principal has nick && principal.nick == "al""#,
        r#"if principal has nick then principal.nick == "al" else false"#,
        r#"principal.hasTag("t") && principal.getTag("t") > 1"#,
        r#"principal has nick && principal.nick == "al" && principal.hasTag("t") && principal.getTag("t") > 1"#,
    ] {
        let found = findings_with_semantics(&schema, &policy(when))
            .await
            .unwrap();
        assert!(found.is_empty(), "`{when}`: {found:?}");
    }
    // the guarded scope is itself always true: that is the finding, the
    // guard having folded away with it
    let found = findings_with_semantics(
        &schema,
        &policy("principal has nick && principal.nick == principal.nick"),
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        vec![(
            "principal.nick == principal.nick".to_string(),
            FindingKind::NeverFalse
        )]
    );
}

/// A guard the assumptions make always true is still reported as such when
/// nothing after it relies on it — the attribute (or tag) is not accessed in
/// the scope it guards, wherever the guard sits.
#[tokio::test]
async fn unneeded_present_guard_is_reported() {
    let schema = present_schema();
    let policy = |when: &str| format!("permit(principal, action, resource) when {{ {when} }};");
    for (when, guard) in [
        ("principal has nick && principal.a", "principal has nick"),
        ("principal.a && principal has nick", "principal has nick"),
        (
            "if principal has nick then principal.a else principal.b",
            "principal has nick",
        ),
        (
            r#"principal.hasTag("t") && principal.a"#,
            r#"principal.hasTag("t")"#,
        ),
    ] {
        let found = findings_with_semantics(&schema, &policy(when))
            .await
            .unwrap();
        assert_eq!(
            found,
            vec![(guard.to_string(), FindingKind::NeverFalse)],
            "`{when}`"
        );
    }
}
