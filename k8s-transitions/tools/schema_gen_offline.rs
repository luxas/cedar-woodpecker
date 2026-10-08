//! Offline variant of schema_gen: reads the APIResourceList discovery fixtures
//! (api/discovery/*.json) and OpenAPI v3 documents (api/openapi-spec/v3/*.json)
//! of a kubernetes source tree instead of a live cluster. Usage:
//!   schema_gen_offline <kubernetes-src-dir> <group/version>...
//! where core is written "v1" and other groups "apps/v1".
use anyhow::{Context, Result};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResourceList;
use kubernetes_cedar_authorizer::schema::{build_base_schema, build_schema_for_gv, CedarGroupVersion};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let k8s = &args[1];
    let mut cedar_schema = build_base_schema(Vec::new())?;
    for gvs in &args[2..] {
        let (gv, stem) = match gvs.split_once('/') {
            None => (CedarGroupVersion::new("".to_string(), gvs.clone())?, format!("api__{gvs}")),
            Some((g, v)) => (CedarGroupVersion::new(g.to_string(), v.to_string())?, format!("apis__{g}__{v}")),
        };
        let disc = format!("{k8s}/api/discovery/{stem}.json");
        let oapi = format!("{k8s}/api/openapi-spec/v3/{stem}_openapi.json");
        eprintln!("{gvs}: {disc} + {oapi}");
        let arl: APIResourceList = serde_json::from_str(&std::fs::read_to_string(&disc).with_context(|| disc.clone())?)?;
        let spec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&oapi).with_context(|| oapi.clone())?)?;
        build_schema_for_gv(&mut cedar_schema, &gv, &arl, &spec).with_context(|| gvs.clone())?;
    }
    println!("{}", cedar_schema.to_cedarschema()?);
    Ok(())
}
