use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

#[path = "archon_learning_migrate/relations.rs"]
mod relations;

use relations::{ALL_RELATION_SETS, RelationSpec};

const LEARNING_DB: &str = "learning-state.db";
const LEGACY_PIPELINE_DB: &str = "learning.db";
const SHARED_EVIDENCE_DB: &str = "archon-data.db";

fn main() -> Result<()> {
    let cwd = parse_cwd_arg()?;
    let archon_dir = cwd.join(".archon");
    fs::create_dir_all(&archon_dir).context("create .archon directory")?;

    let target_path = archon_dir.join(LEARNING_DB);
    if let Some(backup) = backup_existing_target(&target_path)? {
        println!("Backed up existing target: {}", backup.display());
    }

    let target = open_db("sqlite", &target_path).context("open target learning-state.db")?;
    archon_learning::schema::ensure_learning_schema(&target)?;
    archon_pipeline::learning::schema::initialize_learning_schemas(&target)?;

    let mut total = Totals::default();
    migrate_source(
        "legacy RocksDB learning.db",
        "newrocksdb",
        &archon_dir.join(LEGACY_PIPELINE_DB),
        &target,
        &mut total,
    )?;
    migrate_source(
        "shared SQLite archon-data.db",
        "sqlite",
        &archon_dir.join(SHARED_EVIDENCE_DB),
        &target,
        &mut total,
    )?;

    println!(
        "Migration complete: copied={}, skipped_existing={}, missing_relations={}",
        total.copied, total.skipped, total.missing_relations
    );
    println!("Target: {}", target_path.display());
    Ok(())
}

fn parse_cwd_arg() -> Result<PathBuf> {
    let mut cwd = env::current_dir().context("read current directory")?;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cwd" => {
                cwd = PathBuf::from(args.next().context("--cwd requires a path")?);
            }
            "--help" | "-h" => {
                println!("Usage: archon_learning_migrate [--cwd <project-dir>]");
                std::process::exit(0);
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    Ok(cwd)
}

fn backup_existing_target(target_path: &Path) -> Result<Option<PathBuf>> {
    if !target_path.exists() {
        return Ok(None);
    }
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before Unix epoch")?
        .as_secs();
    let backup = target_path.with_extension(format!("db.backup-{suffix}"));
    fs::copy(target_path, &backup)
        .with_context(|| format!("backup {} to {}", target_path.display(), backup.display()))?;
    Ok(Some(backup))
}

fn migrate_source(
    label: &'static str,
    engine: &'static str,
    path: &Path,
    target: &DbInstance,
    total: &mut Totals,
) -> Result<()> {
    if !path.exists() {
        println!("Skipping {label}: not found ({})", path.display());
        return Ok(());
    }

    let source = match open_db(engine, path) {
        Ok(db) => db,
        Err(error) if is_database_locked(&error) => {
            println!("Skipping {label}: database is locked ({})", path.display());
            return Ok(());
        }
        Err(error) => return Err(error).with_context(|| format!("open {label}")),
    };

    let mut report = Totals::default();
    for relation_set in ALL_RELATION_SETS {
        for spec in *relation_set {
            let outcome = match copy_relation(&source, target, spec) {
                Ok(outcome) => outcome,
                Err(error) if is_database_locked(&error) => {
                    merge_totals(total, &report);
                    println!(
                        "Stopping {label}: database locked while reading \
                         (partial copied={}, skipped_existing={})",
                        report.copied, report.skipped
                    );
                    return Ok(());
                }
                Err(error) => return Err(error).with_context(|| spec.name.to_string()),
            };
            report.copied += outcome.copied;
            report.skipped += outcome.skipped;
            report.missing_relations += usize::from(outcome.missing_source);
        }
    }

    merge_totals(total, &report);
    println!(
        "{label}: copied={}, skipped_existing={}, missing_relations={}",
        report.copied, report.skipped, report.missing_relations
    );
    Ok(())
}

fn merge_totals(total: &mut Totals, report: &Totals) {
    total.copied += report.copied;
    total.skipped += report.skipped;
    total.missing_relations += report.missing_relations;
}

fn open_db(engine: &str, path: &Path) -> Result<DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new(engine, &path_str, "").map_err(|error| anyhow::anyhow!("{error}"))
}

#[derive(Default)]
struct Totals {
    copied: usize,
    skipped: usize,
    missing_relations: usize,
}

struct CopyOutcome {
    copied: usize,
    skipped: usize,
    missing_source: bool,
}

fn copy_relation(
    source: &DbInstance,
    target: &DbInstance,
    spec: &RelationSpec,
) -> Result<CopyOutcome> {
    let Some(source_rows) = read_relation(source, spec)? else {
        return Ok(CopyOutcome {
            copied: 0,
            skipped: 0,
            missing_source: true,
        });
    };
    if source_rows.rows.is_empty() {
        return Ok(CopyOutcome {
            copied: 0,
            skipped: 0,
            missing_source: false,
        });
    }

    let mut existing = relation_keys(target, spec)?;
    let mut copied = 0;
    let mut skipped = 0;
    for row in source_rows.rows {
        let key = row_key(spec, &row);
        if existing.contains(&key) {
            skipped += 1;
            continue;
        }
        put_relation_row(target, spec, &row)?;
        existing.insert(key);
        copied += 1;
    }
    Ok(CopyOutcome {
        copied,
        skipped,
        missing_source: false,
    })
}

fn read_relation(db: &DbInstance, spec: &RelationSpec) -> Result<Option<NamedRows>> {
    let columns = spec.columns.join(", ");
    let script = format!("?[{columns}] := *{}{{{columns}}}", spec.name);
    match db.run_script(&script, Default::default(), ScriptMutability::Immutable) {
        Ok(rows) => Ok(Some(rows)),
        Err(error) if relation_missing(&error) => Ok(None),
        Err(error) => Err(anyhow::anyhow!("{error}")),
    }
}

fn relation_keys(db: &DbInstance, spec: &RelationSpec) -> Result<BTreeSet<String>> {
    let rows = read_relation(db, spec)?.with_context(|| {
        format!(
            "target relation {} is missing; target schema was not initialized",
            spec.name
        )
    })?;
    Ok(rows.rows.iter().map(|row| row_key(spec, row)).collect())
}

fn row_key(spec: &RelationSpec, row: &[DataValue]) -> String {
    spec.keys
        .iter()
        .map(|key| {
            let index = spec
                .columns
                .iter()
                .position(|column| column == key)
                .unwrap_or(0);
            format!("{:?}", row.get(index))
        })
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn put_relation_row(db: &DbInstance, spec: &RelationSpec, row: &[DataValue]) -> Result<()> {
    let params = row
        .iter()
        .enumerate()
        .map(|(index, value)| (format!("p{index}"), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let columns = spec.columns.join(", ");
    let values = (0..spec.columns.len())
        .map(|index| format!("$p{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let value_columns = spec.columns[spec.keys.len()..].join(", ");
    let script = format!(
        "?[{columns}] <- [[{values}]] :put {} {{ {} => {value_columns} }}",
        spec.name,
        spec.keys.join(", ")
    );
    db.run_script(&script, params, ScriptMutability::Mutable)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn relation_missing(error: &cozo::Error) -> bool {
    error
        .to_string()
        .contains(archon_learning::errors::COZO_RELATION_NOT_FOUND)
}

fn is_database_locked(error: &anyhow::Error) -> bool {
    error.to_string().contains("database is locked")
}
