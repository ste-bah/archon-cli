//! `vec_page_images`: creation, and the migration of a relation sized for an
//! older embedding dimension. Store errors are reported; only an absent
//! relation reads as "nothing to migrate".
use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

use super::run_create;

pub(super) fn ensure_vec_page_images(db: &DbInstance, dim: usize) -> Result<()> {
    // Migration: a DB created by a pre-CLIP build sized `vec_page_images` to the TEXT embedding
    // dimension. The `:create` below is a no-op when the relation already exists, so a stale
    // 768-dim relation would silently reject 512-dim CLIP image vectors forever (insert fails →
    // only a warning). If the existing relation's dim differs, drop it (and its HNSW index) so
    // it is recreated at the correct image dim. It only ever holds image vectors — none exist on
    // such old DBs — so the drop is safe; embeddings regenerate on (re-)ingest.
    if let Some(existing) = existing_vec_page_images_dim(db)?
        && existing != dim
    {
        // Drop the HNSW index before the relation (a relation with a live index
        // can't be removed). A relation left without its index by an earlier
        // interrupted migration has nothing to drop.
        if vec_page_images_has_index(db)? {
            run_migration_step(
                db,
                "::hnsw drop vec_page_images:page_image_embedding_idx",
                "vec_page_images index drop",
            )?;
        }
        run_migration_step(
            db,
            "::remove vec_page_images",
            "vec_page_images dim migration",
        )?;
    }

    let create_rel = format!(
        ":create vec_page_images {{
            page_id: String
            =>
            embedding: <F32; {dim}>,
            provider: String
        }}"
    );
    run_create(db, &create_rel)?;

    let create_idx = format!(
        "::hnsw create vec_page_images:page_image_embedding_idx {{
            dim: {dim},
            m: 50,
            dtype: F32,
            fields: [embedding],
            distance: Cosine,
            ef_construction: 200
        }}"
    );
    run_create(db, &create_idx)?;

    Ok(())
}

fn run_migration_step(db: &DbInstance, script: &str, context: &str) -> Result<()> {
    crate::cozo_retry::run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        context,
    )
    .map(|_| ())
    .map_err(|error| error.context(format!("{context} failed")))
}

/// Read a schema introspection script, or `None` when the relation is absent.
/// Any other error (busy store, I/O, permanent lock) is returned, never read
/// as "absent".
fn introspect(db: &DbInstance, script: &str, context: &str) -> Result<Option<cozo::NamedRows>> {
    match crate::cozo_retry::run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Immutable,
        context,
    ) {
        Ok(rows) => Ok(Some(rows)),
        Err(error) if is_relation_absent(&error) => Ok(None),
        Err(error) => Err(error.context(format!("{context} failed"))),
    }
}

fn is_relation_absent(error: &anyhow::Error) -> bool {
    archon_cozo::StoreBusy::find(error.as_ref()).is_none()
        && format!("{error:#}").contains(crate::errors::COZO_RELATION_NOT_FOUND)
}

fn vec_page_images_has_index(db: &DbInstance) -> Result<bool> {
    let Some(rows) = introspect(db, "::indices vec_page_images", "vec_page_images indices")? else {
        return Ok(false);
    };
    Ok(rows.rows.iter().flatten().any(|cell| {
        cell.get_str()
            .is_some_and(|name| name.contains("page_image_embedding_idx"))
    }))
}

/// The embedding dimension of an existing `vec_page_images` relation via
/// `::columns`: `None` only when the relation does not exist. A relation whose
/// vector column cannot be read is an error, not "absent".
pub(super) fn existing_vec_page_images_dim(db: &DbInstance) -> Result<Option<usize>> {
    let Some(result) = introspect(
        db,
        "::columns vec_page_images",
        "existing vec page images dim",
    )?
    else {
        return Ok(None);
    };
    for row in &result.rows {
        for cell in row {
            let Some(text) = cell.get_str() else { continue };
            // The embedding column's type renders as "<F32; N>" — take the digits after ';'.
            if text.contains("F32")
                && let Some(semi) = text.find(';')
            {
                let digits: String = text[semi + 1..]
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(parsed) = digits.parse::<usize>() {
                    return Ok(Some(parsed));
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "vec_page_images exists but its embedding dimension could not be read from ::columns"
    ))
}
