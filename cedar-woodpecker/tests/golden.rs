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

//! Every test of the crate is a case directory under `testdata/<suite>/` with
//! its full inputs and an expected output; this driver runs every case of
//! every suite and compares. `UPDATE_GOLDEN=1 cargo test` rewrites the
//! expectations; review them like code. The `e2e` suite needs `cvc5`.

#![expect(clippy::unwrap_used, clippy::panic, reason = "test code")]

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use cedar_policy::Expression;
use cedar_policy_core::ast::Expr;
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;
use cedar_woodpecker::cubes::{cubes_to_cedar, cubes_to_json};
use cedar_woodpecker::escalate::Setup;
use cedar_woodpecker::{
    equiv, escalate, evaluator_with, expr, extend_schema, parse_entities, parse_policies, sets,
    source_cubes, Budgets, Error, LoadedSchema, Transition,
};
use smol_str::SmolStr;

/// The case directories of a suite, in name order.
fn cases(suite: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(suite);
    let mut cases: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no cases in {}", dir.display());
    cases
}

fn read(case: &Path, file: &str) -> String {
    fs::read_to_string(case.join(file)).unwrap_or_else(|e| panic!("{}/{file}: {e}", case.display()))
}

fn read_opt(case: &Path, file: &str) -> Option<String> {
    fs::read_to_string(case.join(file)).ok()
}

/// Compares `actual` with the case's expected `file`, or rewrites it.
fn check(case: &Path, file: &str, actual: &str) {
    let path = case.join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (run with UPDATE_GOLDEN=1 to create it)",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "{} differs from the expected output (UPDATE_GOLDEN=1 rewrites it):\n--- expected\n{expected}\n--- actual\n{actual}",
        path.display()
    );
}

fn parse(text: &str) -> Expr {
    Expression::from_str(text)
        .unwrap_or_else(|e| panic!("`{text}`: {e}"))
        .as_ref()
        .clone()
}

/// The header lines `key: value` of an input file and the rest.
fn header(text: &str, keys: &[&str]) -> (Vec<(String, String)>, Vec<String>) {
    let mut headers = Vec::new();
    let mut rest = Vec::new();
    for line in text.lines() {
        let header = keys
            .iter()
            .find_map(|k| line.strip_prefix(&format!("{k}: ")).map(|v| (*k, v)))
            .filter(|_| rest.is_empty());
        match header {
            Some((k, v)) => headers.push((k.to_string(), v.trim().to_string())),
            None if line.trim().is_empty() => {}
            None => rest.push(line.to_string()),
        }
    }
    (headers, rest)
}

fn header_value<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn removed(headers: &[(String, String)]) -> Vec<SmolStr> {
    header_value(headers, "removed")
        .unwrap_or("")
        .split_whitespace()
        .map(SmolStr::from)
        .collect()
}

/// The `e2e` suite: the whole pipeline on a schema, policies and
/// transitions, as a Cedar file (the cubes and the synthesized policies, with
/// comments) and as JSON.
async fn run_e2e(case: &Path) -> Result<(String, serde_json::Value), Error> {
    let schema = LoadedSchema::parse(&read(case, "schema.cedarschema"), false)?;
    let policies = parse_policies(&read(case, "input.cedar"))?;
    let entities = read_opt(case, "entities.json")
        .map(|t| parse_entities(&t, &schema.schema))
        .transpose()?;
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap())?;
    let mut evaluator = evaluator_with(
        compiler,
        &schema.schema,
        &schema.assumptions,
        entities.as_ref(),
    )
    .await?;
    let cubes = source_cubes(
        &policies,
        &schema.schema,
        &mut evaluator,
        Budgets::default(),
    )
    .await?;
    let mut text = format!("// == cubes ==\n{}", cubes_to_cedar(&cubes)?);
    let mut json = serde_json::json!({ "cubes": cubes_to_json(&cubes) });
    if let Some(transitions) = read_opt(case, "transitions.json") {
        let transitions = Transition::parse_json(&transitions)?;
        let setup = Setup {
            schema: &schema.schema,
            schema_json: &schema.json,
            assumptions: &schema.assumptions,
            entities: entities.as_ref(),
            budgets: Budgets::default(),
        };
        let (escalations, _) =
            escalate(&cubes, &transitions, setup, evaluator.into_compiler()).await?;
        text.push_str("// == escalations ==\n");
        if escalations.is_empty() {
            text.push_str("// (none)\n");
        }
        for e in &escalations {
            text.push_str(&e.to_text());
        }
        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "escalations".to_string(),
                escalations.iter().map(|e| e.to_json()).collect(),
            );
        }
    }
    Ok((text, json))
}

#[tokio::test]
async fn e2e() {
    for case in cases("e2e") {
        let (text, json) = match run_e2e(&case).await {
            Ok(out) => out,
            Err(e) => (
                format!(
                    "// == error ==\n{}\n",
                    e.to_string()
                        .lines()
                        .map(|l| format!("// {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
                serde_json::json!({ "error": e.to_string() }),
            ),
        };
        check(&case, "output.cedar", &text);
        check(
            &case,
            "output.json",
            &format!("{}\n", serde_json::to_string_pretty(&json).unwrap()),
        );
    }
}

/// The `equiv` suite: `removed: <attrs>` then one conjunct per line; the
/// rewritten conjuncts.
#[test]
fn equiv() {
    for case in cases("equiv") {
        let (headers, lines) = header(&read(&case, "input.txt"), &["removed"]);
        let conjuncts: Vec<Expr> = lines.iter().map(|l| parse(l)).collect();
        let out = equiv::rewrite(&conjuncts, &removed(&headers));
        let text = out
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        check(&case, "expected.txt", &text);
    }
}

/// The `sets` suite: `removed: <attrs>` then one conjunct per line; the
/// conjuncts after the set and `in` rewrites.
#[test]
fn sets_rewrite() {
    for case in cases("sets") {
        let (headers, lines) = header(&read(&case, "input.txt"), &["removed"]);
        let conjuncts: Vec<Expr> = lines.iter().map(|l| parse(l)).collect();
        let out = sets::rewrite(&conjuncts, &removed(&headers));
        let text = out
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        check(&case, "expected.txt", &text);
    }
}

/// The `expr` suite: `op: <op>` (with `from`/`to`/`removed` where the op
/// takes them) then the expression(s); the op's result.
#[test]
fn expr_ops() {
    for case in cases("expr") {
        let (headers, lines) = header(&read(&case, "input.txt"), &["op", "from", "to", "removed"]);
        let op = header_value(&headers, "op").expect("op");
        let first = || parse(lines.first().expect("an expression"));
        let out = match op {
            "rename" => expr::rename_vars(
                &first(),
                &expr::context_attr("action1"),
                &expr::context_attr("resource1"),
                &expr::context_attr("context1"),
            )
            .to_string(),
            "simplify" => expr::simplify(&first()).to_string(),
            "replace" => expr::replace(
                &first(),
                &parse(header_value(&headers, "from").expect("from")),
                &parse(header_value(&headers, "to").expect("to")),
            )
            .to_string(),
            "mentions" => expr::mentions_context_attrs(&first(), &removed(&headers)).to_string(),
            "constant" => expr::is_constant(&first()).to_string(),
            "and_chain" | "or_chain" => {
                let exprs = lines.iter().map(|l| parse(l));
                let chain = if op == "and_chain" {
                    expr::and_chain(exprs)
                } else {
                    expr::or_chain(exprs)
                };
                chain.map_or_else(|| "(none)".to_string(), |e| e.to_string())
            }
            other => panic!("{}: unknown op `{other}`", case.display()),
        };
        check(&case, "expected.txt", &format!("{out}\n"));
    }
}

/// The `schema` suite: a schema and a transition; the extended schema, or
/// the error.
#[test]
fn schema() {
    for case in cases("schema") {
        let loaded = LoadedSchema::parse(&read(&case, "schema.cedarschema"), false).unwrap();
        let transition = Transition::parse_json(&read(&case, "transition.json"))
            .unwrap()
            .remove(0);
        match extend_schema(&loaded.json, &transition) {
            Ok(ext) => {
                check(
                    &case,
                    "expected.cedarschema",
                    &ext.fragment.to_cedarschema().unwrap(),
                );
            }
            Err(e) => check(&case, "expected.txt", &format!("error: {e}\n")),
        }
    }
}

/// The `transition` suite: a transition file; its listing, or the error.
#[test]
fn transition() {
    for case in cases("transition") {
        let text = match Transition::parse_json(&read(&case, "transitions.json")) {
            Ok(transitions) => {
                transitions
                    .iter()
                    .map(|t| {
                        let principal = t.principal.as_ref().map_or_else(
                            || "(every applicable type)".to_string(),
                            |ps| {
                                ps.iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
                        );
                        let sources = t
                            .sources
                            .iter()
                            .map(|s| format!("{} on {}", s.action, s.resource))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let added = t
                            .added_attrs()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ");
                        [
                            t.name.clone(),
                            format!("  principal: {principal}"),
                            format!("  sources: {sources}"),
                            format!("  target: {} on {}", t.target.action, t.target.resource),
                            format!("  when: {}", t.when),
                            format!("  added: {added}"),
                        ]
                        .join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            }
            Err(e) => format!("error: {e}\n"),
        };
        check(&case, "expected.txt", &text);
    }
}
