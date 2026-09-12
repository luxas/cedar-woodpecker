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

//! `cedar-woodpecker`: the privilege-escalation finder as a command. Needs
//! `cvc5` on `PATH` (or in `CVC5`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cedar_policy_symcc::dnf::{DEFAULT_MAX_CUBES, DEFAULT_MAX_SPLIT_NODES};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;
use cedar_woodpecker::cubes::{cubes_to_cedar, cubes_to_json};
use cedar_woodpecker::escalate::Setup;
use cedar_woodpecker::{
    escalate, evaluator_with, parse_entities, parse_policies, source_cubes, Budgets, Error,
    LoadedSchema, Transition,
};
use clap::{Args, Parser, Subcommand};

/// Finds privilege-escalation paths in a Cedar policy set and synthesizes
/// the policies they imply.
#[derive(Parser, Debug)]
#[command(name = "cedar-woodpecker", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// The explicit permissions: the allow-only DNF cubes that can be true,
    /// per request environment.
    Cubes(CubesArgs),
    /// The escalation paths of the transitions and the policies they imply.
    Escalate(EscalateArgs),
}

/// The inputs every command takes.
#[derive(Args, Debug)]
struct Inputs {
    /// The schema (Cedar syntax, or JSON with a `.json` extension).
    #[arg(long)]
    schema: PathBuf,
    /// The policies (Cedar syntax); stdin when absent.
    #[arg(long)]
    policies: Option<PathBuf>,
    /// Entity data (JSON) to assume.
    #[arg(long)]
    entities: Option<PathBuf>,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
    /// The node budget of an atom split.
    #[arg(long, default_value_t = DEFAULT_MAX_SPLIT_NODES)]
    max_split_nodes: usize,
    /// The cube budget of a policy.
    #[arg(long, default_value_t = DEFAULT_MAX_CUBES)]
    max_cubes: usize,
}

#[derive(Args, Debug)]
struct CubesArgs {
    #[command(flatten)]
    inputs: Inputs,
}

#[derive(Args, Debug)]
struct EscalateArgs {
    #[command(flatten)]
    inputs: Inputs,
    /// The transition functions (JSON).
    #[arg(long)]
    transitions: PathBuf,
}

fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path)
        .map_err(|e| Error::Internal(format!("cannot read {}: {e}", path.display())))
}

fn read_policies(path: Option<&Path>) -> Result<String, Error> {
    match path {
        Some(path) => read(path),
        None => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| Error::Internal(format!("cannot read stdin: {e}")))?;
            Ok(text)
        }
    }
}

struct Loaded {
    schema: LoadedSchema,
    policies: cedar_policy::PolicySet,
    entities: Option<cedar_policy::Entities>,
    budgets: Budgets,
}

fn load(inputs: &Inputs) -> Result<Loaded, Error> {
    let schema_text = read(&inputs.schema)?;
    let json = inputs.schema.extension().is_some_and(|e| e == "json");
    let schema = LoadedSchema::parse(&schema_text, json)?;
    let policies = parse_policies(&read_policies(inputs.policies.as_deref())?)?;
    let entities = inputs
        .entities
        .as_deref()
        .map(|p| read(p).and_then(|t| parse_entities(&t, &schema.schema)))
        .transpose()?;
    Ok(Loaded {
        schema,
        policies,
        entities,
        budgets: Budgets {
            max_split_nodes: inputs.max_split_nodes,
            max_cubes: inputs.max_cubes,
        },
    })
}

async fn run(command: Command) -> Result<String, Error> {
    let inputs = match &command {
        Command::Cubes(a) => &a.inputs,
        Command::Escalate(a) => &a.inputs,
    };
    let loaded = load(inputs)?;
    let solver = LocalSolver::cvc5().map_err(|e| {
        Error::Internal(format!(
            "cannot start cvc5 (set CVC5 or put cvc5 on PATH): {e}"
        ))
    })?;
    let compiler = CedarSymCompiler::new(solver)?;
    let mut evaluator = evaluator_with(
        compiler,
        &loaded.schema.schema,
        &loaded.schema.assumptions,
        loaded.entities.as_ref(),
    )
    .await?;
    let cubes = source_cubes(
        &loaded.policies,
        &loaded.schema.schema,
        &mut evaluator,
        loaded.budgets,
    )
    .await?;
    match command {
        Command::Cubes(_) => {
            if inputs.json {
                Ok(serde_json::to_string_pretty(&cubes_to_json(&cubes)).unwrap_or_default())
            } else {
                cubes_to_cedar(&cubes)
            }
        }
        Command::Escalate(args) => {
            let transitions = Transition::parse_json(&read(&args.transitions)?)?;
            let setup = Setup {
                schema: &loaded.schema.schema,
                schema_json: &loaded.schema.json,
                assumptions: &loaded.schema.assumptions,
                entities: loaded.entities.as_ref(),
                budgets: loaded.budgets,
            };
            let (escalations, _) =
                escalate(&cubes, &transitions, setup, evaluator.into_compiler()).await?;
            Ok(if args.inputs.json {
                let all: Vec<_> = escalations.iter().map(|e| e.to_json()).collect();
                serde_json::to_string_pretty(&all).unwrap_or_default()
            } else if escalations.is_empty() {
                "// no escalation paths\n".to_string()
            } else {
                escalations
                    .iter()
                    .map(|e| e.to_text())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli.command)) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
