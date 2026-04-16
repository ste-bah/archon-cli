use std::sync::Arc;
use std::path::PathBuf;

use archon_core::agents::catalog::{AgentFilter, DiscoveryCatalog, FilterLogic};
use archon_core::agents::discovery::local::LocalDiscoverySource;
use archon_core::agents::discovery::remote::RemoteDiscoverySource;
use archon_core::agents::schema::AgentSchemaValidator;

fn build_catalog(working_dir: &PathBuf) -> DiscoveryCatalog {
    let validator = Arc::new(AgentSchemaValidator::new().expect("schema compile"));
    let catalog = DiscoveryCatalog::new();
    let agents_dir = working_dir.join(".archon/agents");
    if agents_dir.is_dir() {
        let source = LocalDiscoverySource::new(agents_dir, validator);
        if let Ok(report) = source.load_all(&catalog) {
            tracing::debug!(loaded = report.loaded, invalid = report.invalid, "agent scan");
        }
    }
    catalog
}

pub(crate) async fn handle_agent_list(
    include_invalid: bool,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let catalog = build_catalog(working_dir);
    let filter = AgentFilter {
        include_invalid,
        ..Default::default()
    };
    let results = catalog.list(&filter);
    println!("{:<30} {:<10} {:<15} {}", "NAME", "VERSION", "CATEGORY", "DESCRIPTION");
    println!("{}", "-".repeat(80));
    for meta in &results {
        let desc = meta.description.lines().next().unwrap_or("");
        let desc_trunc: String = desc.chars().take(40).collect();
        println!("{:<30} {:<10} {:<15} {}", meta.name, meta.version, meta.category, desc_trunc);
    }
    println!("\n{} agents", results.len());
    Ok(())
}

pub(crate) async fn handle_agent_search(
    tags: Vec<String>,
    capabilities: Vec<String>,
    name_pattern: Option<String>,
    version: Option<String>,
    logic: String,
    include_invalid: bool,
    registry_url: Option<String>,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let validator = Arc::new(AgentSchemaValidator::new().expect("schema compile"));
    let catalog = DiscoveryCatalog::new();
    let agents_dir = working_dir.join(".archon/agents");
    if agents_dir.is_dir() {
        let source = LocalDiscoverySource::new(agents_dir, validator.clone());
        if let Ok(report) = source.load_all(&catalog) {
            tracing::debug!(loaded = report.loaded, invalid = report.invalid, "local scan");
        }
    }

    if let Some(url) = registry_url {
        let remote = RemoteDiscoverySource::new(url, 3600, validator);
        if let Err(e) = remote.load_all(&catalog).await {
            eprintln!("warning: remote registry fetch failed: {e}");
        }
    }

    let total = catalog.len();
    let filter = AgentFilter {
        tags,
        capabilities,
        name_pattern: match name_pattern {
            Some(p) => match globset::Glob::new(&p) {
                Ok(g) => Some(g),
                Err(e) => {
                    eprintln!("error: invalid name pattern: {e}");
                    std::process::exit(1);
                }
            },
            None => None,
        },
        version_req: match version {
            Some(v) => match semver::VersionReq::parse(&v) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("error: invalid version requirement: {e}");
                    std::process::exit(1);
                }
            },
            None => None,
        },
        logic: if logic.eq_ignore_ascii_case("or") {
            FilterLogic::Or
        } else {
            FilterLogic::And
        },
        include_invalid,
    };
    let results = catalog.list(&filter);
    println!("{:<30} {:<10} {:<15} {}", "NAME", "VERSION", "CATEGORY", "DESCRIPTION");
    println!("{}", "-".repeat(80));
    for meta in &results {
        let desc = meta.description.lines().next().unwrap_or("");
        let desc_trunc: String = desc.chars().take(40).collect();
        println!("{:<30} {:<10} {:<15} {}", meta.name, meta.version, meta.category, desc_trunc);
    }
    println!("\n{} agents (filtered from {})", results.len(), total);
    Ok(())
}

pub(crate) async fn handle_agent_info(
    name: String,
    version: Option<String>,
    json: bool,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    use archon_core::agents::catalog::DiscoveryError;

    let validator = Arc::new(AgentSchemaValidator::new().expect("schema compile"));
    let catalog = DiscoveryCatalog::new();
    let agents_dir = working_dir.join(".archon/agents");
    if agents_dir.is_dir() {
        let source = LocalDiscoverySource::new(agents_dir, validator);
        let _ = source.load_all(&catalog);
    }

    let version_req = match version {
        Some(v) => match semver::VersionReq::parse(&v) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("error: invalid version requirement: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    let info = match catalog.info(&name, version_req.as_ref()) {
        Ok(info) => info,
        Err(DiscoveryError::AgentNotFound { name, suggestions }) => {
            eprintln!(
                "Agent '{name}' not found. Did you mean: {}?",
                suggestions.join(", ")
            );
            std::process::exit(12);
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&info).unwrap_or_default());
    } else {
        let all_versions: Vec<String> = info.all_versions.iter().map(|v| v.to_string()).collect();
        println!("Name:         {}", info.selected.name);
        println!("Version:      {} (of: {})", info.selected.version, all_versions.join(", "));
        println!("Category:     {}", info.selected.category);
        println!("Description:  {}", info.selected.description);
        println!("Tags:         {}", info.selected.tags.join(", "));
        println!("Capabilities: {}", info.selected.capabilities.join(", "));
        println!(
            "Resources:    cpu={} memory={}MB timeout={}s",
            info.selected.resource_requirements.cpu,
            info.selected.resource_requirements.memory_mb,
            info.selected.resource_requirements.timeout_sec
        );
        println!("Source:       {:?} @ {:?}", info.selected.source_kind, info.selected.source_path);
        println!("State:        {:?}", info.selected.state);
        if !info.dependency_graph.is_empty() {
            println!("Dependencies:");
            for (dep_name, dep_ver) in &info.dependency_graph {
                println!("  - {dep_name}@{dep_ver}");
            }
        }
        println!(
            "Input schema:  {}",
            serde_json::to_string(&info.selected.input_schema).unwrap_or_default()
        );
        println!(
            "Output schema: {}",
            serde_json::to_string(&info.selected.output_schema).unwrap_or_default()
        );
    }
    Ok(())
}
