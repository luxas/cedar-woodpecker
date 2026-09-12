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

//! Integration tests for `cedar lint`. They need cvc5 on `PATH` (or in
//! `CVC5`) and the `analyze` feature.

#![cfg(feature = "analyze")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::indexing_slicing, reason = "tests")]
#![allow(clippy::string_slice, reason = "tests")]

use assert_cmd::cargo;
use predicates::prelude::PredicateBooleanExt;
use std::io::Write;
use tempfile::NamedTempFile;

/// `nick` is always present on a `User`, the schema says.
const SCHEMA: &str = r#"
@semantics("if principal is User then principal has nick else true")
entity User { active: Bool, nick?: String };
entity Doc;
action view appliesTo { principal: User, resource: Doc };
"#;

/// The `has` guards nothing after it: always true, a finding.
const POLICY: &str =
    "permit(principal, action, resource) when { principal has nick && principal.active };\n";

fn write_temp(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

#[test]
fn json_findings_for_editors() {
    let schema = write_temp(SCHEMA);
    let policy = write_temp(POLICY);
    let output = cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let findings: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let findings = findings.as_array().unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["kind"], "never-false");
    let offset = usize::try_from(findings[0]["offset"].as_u64().unwrap()).unwrap();
    let length = usize::try_from(findings[0]["length"].as_u64().unwrap()).unwrap();
    assert_eq!(&POLICY[offset..offset + length], "principal has nick");
}

#[test]
fn policies_from_stdin() {
    let schema = write_temp(SCHEMA);
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--json")
        .write_stdin(POLICY)
        .assert()
        .success()
        .stdout(predicates::str::contains("\"kind\":\"never-false\""));
}

#[test]
fn text_report_with_source() {
    let schema = write_temp(SCHEMA);
    let policy = write_temp(POLICY);
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .assert()
        .success()
        .stdout(
            predicates::str::contains("is always true")
                .and(predicates::str::contains("never false"))
                .and(predicates::str::contains("principal has nick"))
                .and(predicates::str::contains("1 finding(s)")),
        );
}

#[test]
fn clean_policy_has_no_findings() {
    let schema = write_temp(SCHEMA);
    let policy = write_temp("permit(principal, action, resource) when { principal.active };\n");
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("no findings"));
}

#[test]
fn deny_findings_is_a_validation_failure() {
    let schema = write_temp(SCHEMA);
    let policy = write_temp(POLICY);
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .arg("--deny-findings")
        .assert()
        .code(3);
}

#[test]
fn unparsable_policies_fail() {
    let schema = write_temp(SCHEMA);
    let policy = write_temp("permit(principal, action, resource) when { ;\n");
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .assert()
        .code(1)
        .stderr(predicates::str::contains("do not parse"));
}

/// A rejected `@semantics` annotation is one line on stderr starting with
/// `the @semantics` in JSON mode (what the VS Code extension matches on).
#[test]
fn rejected_semantics_in_json_mode() {
    let schema = write_temp(&SCHEMA.replace(
        "if principal is User then principal has nick else true",
        "principal.nope",
    ));
    let policy = write_temp(POLICY);
    cargo::cargo_bin_cmd!("cedar")
        .arg("lint")
        .arg("--schema")
        .arg(schema.path())
        .arg("--policies")
        .arg(policy.path())
        .arg("--json")
        .assert()
        .code(1)
        .stderr(predicates::str::starts_with("the @semantics assumption on"));
}
