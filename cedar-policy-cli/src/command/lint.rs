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

//! `cedar lint`: the policy simplifier (`cedar_policy_symcc::simplify`) as a
//! command. Every expression of a policy set that is statically true, false
//! or an error in every request environment of the schema — given what is
//! evaluated before it, and the schema's `@semantics` assumptions — is a
//! finding, reported as a warning on its source span, or as JSON for editors.

use std::path::PathBuf;

use cedar_policy_symcc::evaluator::Evaluator;
use cedar_policy_symcc::semantics::collect_semantics;
use cedar_policy_symcc::simplify::{simplify, Finding, FindingKind};
use cedar_policy_symcc::CedarSymCompiler;
use clap::Args;
use miette::{Diagnostic, IntoDiagnostic, NamedSource, Report, Result, SourceSpan, WrapErr};

use super::symcc::initialize_solver;
use crate::{read_from_file_or_stdin, CedarExitCode, SchemaArgs, SchemaFormat};

#[derive(Args, Debug)]
pub struct LintArgs {
    /// Schema args (incorporated by reference)
    #[command(flatten)]
    pub schema: SchemaArgs,
    /// File containing the Cedar policies, in Cedar syntax (the findings point
    /// into it). If not provided, read policies from stdin.
    #[arg(short, long = "policies", value_name = "FILE")]
    pub policies_file: Option<String>,
    /// Path to CVC5 solver executable
    #[arg(long, env = "CVC5")]
    pub cvc5_path: Option<PathBuf>,
    /// Print the findings as a JSON array of `{offset, length, kind, message}`
    /// (byte offsets into the policy document), for editors; errors are then
    /// one line on stderr
    #[arg(long)]
    pub json: bool,
    /// Exit with a validation failure when there is any finding
    #[arg(long)]
    pub deny_findings: bool,
}

/// One finding as a diagnostic on the policy document.
#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("{message}")]
#[diagnostic(severity(Warning))]
struct LintFinding {
    message: String,
    #[source_code]
    src: NamedSource<String>,
    #[label("{label}")]
    span: SourceSpan,
    label: String,
}

fn label(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::NeverTrue => "never true",
        FindingKind::NeverFalse => "never false",
        FindingKind::AlwaysError => "always errors",
    }
}

pub fn lint(args: &LintArgs) -> CedarExitCode {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to initialize async runtime: {e}");
            return CedarExitCode::Failure;
        }
    };
    rt.block_on(async {
        match lint_async(args).await {
            Ok(code) => code,
            Err(e) => {
                if args.json {
                    // one line, the cause chain joined: editors match on it
                    let chain: Vec<String> = e.chain().map(ToString::to_string).collect();
                    eprintln!("{}", chain.join(": "));
                } else {
                    eprintln!("{:?}", e.wrap_err("Lint failed"));
                }
                CedarExitCode::Failure
            }
        }
    })
}

async fn lint_async(args: &LintArgs) -> Result<CedarExitCode> {
    let (schema_src, schema) = args.schema.get_schema_with_source()?;
    let assumptions = collect_semantics(
        &schema_src,
        matches!(args.schema.schema_format, SchemaFormat::Json),
        &schema,
    )
    .into_diagnostic()?;
    let policies = read_from_file_or_stdin(args.policies_file.as_ref(), "policies")?;
    let solver = initialize_solver(args.cvc5_path.as_ref())?;
    let compiler = CedarSymCompiler::new(solver)
        .into_diagnostic()
        .wrap_err("cannot set up the symbolic compiler")?;
    let evaluator = Evaluator::new(compiler, &schema)
        .into_diagnostic()
        .wrap_err("the schema cannot be compiled symbolically")?;
    let (findings, _) = simplify(evaluator, &schema, &policies, &assumptions)
        .await
        .into_diagnostic()?;
    if args.json {
        println!("{}", serde_json::to_string(&findings).into_diagnostic()?);
    } else {
        let name = args
            .policies_file
            .clone()
            .unwrap_or_else(|| "<stdin>".to_string());
        for finding in &findings {
            println!("{:?}", Report::new(diagnostic(finding, &name, &policies)));
        }
        match findings.len() {
            0 => println!("no findings"),
            n => println!("{n} finding(s)"),
        }
    }
    Ok(if args.deny_findings && !findings.is_empty() {
        CedarExitCode::ValidationFailure
    } else {
        CedarExitCode::Success
    })
}

fn diagnostic(finding: &Finding, name: &str, policies: &str) -> LintFinding {
    LintFinding {
        message: finding.message.clone(),
        src: NamedSource::new(name, policies.to_string()),
        span: (finding.offset, finding.length).into(),
        label: label(finding.kind).to_string(),
    }
}
