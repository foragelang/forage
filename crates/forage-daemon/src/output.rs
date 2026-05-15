//! Per-Run output store. One SQLite file per Run (`Run.output`); one
//! table per record type referenced in the recipe; one row per emit
//! event. Schema is derived from the recipe's type catalog at run
//! time — `derive_schema` walks every reachable `emit Foo { … }`,
//! resolves `Foo` against the workspace catalog, and produces a
//! `TableDef` whose columns map the type's declared fields to SQL
//! storage classes.
//!
//! Two columns are appended to every table:
//! - `_scheduled_run_id`: which run produced this row. Lets consumers
//!   trace a record back to its emit cycle.
//! - `_emitted_at`: ms-epoch wall clock of the emit. Cheap timeline.
//!
//! Schema drift between runs is best-effort: `CREATE TABLE IF NOT
//! EXISTS` runs at the start of every cycle, but altered column lists
//! on existing tables aren't migrated. The original plan defers that
//! to a later phase; we surface column-set differences as runtime
//! errors when writes fail instead of silently dropping data.

use std::path::Path;

use forage_core::ast::{FieldType, ForageFile, JSONValue, RecipeType};
use forage_core::TypeCatalog;
use rusqlite::{Connection, ToSql, params_from_iter};

use crate::error::RunError;

/// Trailing metadata column on every output table — synthetic
/// record id assigned by the engine at emit time (`rec-0`, …). What
/// `Ref<T>` field values point at.
const RECORD_ID_COL: &str = "_id";
/// Trailing metadata column on every output table — carries the
/// `ScheduledRun.id` that produced the row.
const SCHEDULED_RUN_ID_COL: &str = "_scheduled_run_id";
/// Trailing metadata column on every output table — ms-epoch when the
/// row was emitted.
const EMITTED_AT_COL: &str = "_emitted_at";

/// Compiled schema for one record type.
///
/// `field_columns` carries `(name, storage_class)` pairs in the order
/// the record type declared them, so insertion order is stable across
/// recipe re-loads.
#[derive(Debug, Clone, PartialEq)]
pub struct TableDef {
    pub name: String,
    pub field_columns: Vec<ColumnDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub storage: ColumnStorage,
    pub optional: bool,
}

/// SQL storage class. Recipe `FieldType` lowers into one of these:
/// `String` → `Text`, `Int` → `Integer`, `Double` → `Real`,
/// `Bool` → `Integer`, anything compound (array / nested record /
/// enum reference) → `Json`. The `Json` columns carry the field
/// serialized as JSON text so consumers can drill in via `json_extract`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnStorage {
    Text,
    Integer,
    Real,
    Json,
}

impl ColumnStorage {
    pub fn sql(self) -> &'static str {
        match self {
            ColumnStorage::Text => "TEXT",
            ColumnStorage::Integer => "INTEGER",
            ColumnStorage::Real => "REAL",
            ColumnStorage::Json => "TEXT",
        }
    }
}

/// Build the schema for every record type emitted by `recipe`. Walks
/// `recipe.body` + browser captures recursively to find every `emit`
/// site, deduplicates by type name, and resolves each name against
/// the merged `catalog`.
///
/// Composition recipes have no `emit` statements of their own — their
/// records arrive via the chain's final stage. When a composition
/// recipe declares `emits T | U | …`, those types pre-create the
/// output-store tables so the chain's writes land somewhere. A
/// composition without an `emits` clause has no schema until the
/// runtime adds tables on first record; that's a future extension,
/// and today the daemon expects the author to declare `emits` on a
/// composition recipe to enable the chain.
///
/// A reachable `emit Foo` whose `Foo` isn't in the catalog is a
/// validation error that should be caught upstream; here we skip it
/// to avoid panicking — the run already failed validation before
/// reaching this point.
pub fn derive_schema(recipe: &ForageFile, catalog: &TypeCatalog) -> Vec<TableDef> {
    let mut emit_types = recipe.emit_types();
    if recipe.body.composition().is_some() {
        if let Some(out) = &recipe.emits {
            for name in &out.types {
                emit_types.insert(name.clone());
            }
        }
    }

    emit_types
        .into_iter()
        .filter_map(|name| {
            let ty = catalog.ty(&name)?;
            Some(table_def_from_type(ty))
        })
        .collect()
}

fn table_def_from_type(ty: &RecipeType) -> TableDef {
    let field_columns = ty
        .fields
        .iter()
        .map(|f| ColumnDef {
            name: f.name.clone(),
            storage: storage_for(&f.ty),
            optional: f.optional,
        })
        .collect();
    TableDef {
        name: ty.name.clone(),
        field_columns,
    }
}

fn storage_for(ty: &FieldType) -> ColumnStorage {
    match ty {
        FieldType::String => ColumnStorage::Text,
        FieldType::Int => ColumnStorage::Integer,
        FieldType::Double => ColumnStorage::Real,
        FieldType::Bool => ColumnStorage::Integer,
        // Arrays, named records, enum references, and typed refs all
        // serialize as JSON text — keeps the column count bounded and
        // the wire format flat. Refs land as `{"_ref": id, "_type":
        // name}` objects per `EvalValue::Ref::into_json`. Consumers
        // wanting structured access can drill via sqlite's
        // `json_extract`.
        FieldType::Array(_) | FieldType::Record(_) | FieldType::EnumRef(_) | FieldType::Ref(_) => {
            ColumnStorage::Json
        }
    }
}

/// Open (and ensure-schema) the output store at `path`. Idempotent:
/// runs `CREATE TABLE IF NOT EXISTS` for every table in `tables`.
pub struct OutputStore {
    conn: Connection,
    tables: Vec<TableDef>,
}

impl OutputStore {
    pub fn open(path: &Path, tables: Vec<TableDef>) -> Result<Self, RunError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(|e| RunError::Output(e.to_string()))?;
        for t in &tables {
            ensure_table(&conn, t).map_err(|e| RunError::Output(e.to_string()))?;
        }
        Ok(Self { conn, tables })
    }

    /// In-memory `OutputStore`. The schema lands the same way `open`
    /// lays it down, but the store evaporates when this value drops —
    /// nothing touches the persistent `.forage/data/<recipe>.sqlite`.
    /// The dev-preset / `--ephemeral` invocation path uses this.
    pub fn ephemeral(tables: Vec<TableDef>) -> Result<Self, RunError> {
        let conn = Connection::open_in_memory().map_err(|e| RunError::Output(e.to_string()))?;
        for t in &tables {
            ensure_table(&conn, t).map_err(|e| RunError::Output(e.to_string()))?;
        }
        Ok(Self { conn, tables })
    }

    /// Begin a write transaction. Caller stages every emit through
    /// `WriteTx::write_record`, then `commit` flushes them atomically.
    pub fn begin_tx(&mut self) -> Result<WriteTx<'_>, RunError> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| RunError::Output(e.to_string()))?;
        Ok(WriteTx {
            tx,
            tables: &self.tables,
        })
    }
}

pub struct WriteTx<'a> {
    tx: rusqlite::Transaction<'a>,
    tables: &'a [TableDef],
}

impl<'a> WriteTx<'a> {
    /// Insert one row. `record_id` is the synthetic `_id` the engine
    /// assigned to the record (what `Ref<T>` fields elsewhere point at).
    /// `fields` is the field map from `Record.fields` (an
    /// `IndexMap<String, JSONValue>`); the writer projects it into the
    /// table's column order, missing optional fields become SQL NULLs,
    /// missing required fields are a write error.
    pub fn write_record(
        &mut self,
        scheduled_run_id: &str,
        emitted_at_ms: i64,
        record_id: &str,
        type_name: &str,
        fields: &indexmap::IndexMap<String, JSONValue>,
    ) -> Result<(), RunError> {
        let table = self
            .tables
            .iter()
            .find(|t| t.name == type_name)
            .ok_or_else(|| {
                RunError::Output(format!(
                    "record type '{type_name}' has no derived table — recipe schema mismatch"
                ))
            })?;
        let mut col_names: Vec<String> = Vec::with_capacity(table.field_columns.len() + 3);
        let mut placeholders: Vec<String> = Vec::with_capacity(table.field_columns.len() + 3);
        let mut values: Vec<Box<dyn ToSql>> = Vec::with_capacity(table.field_columns.len() + 3);
        for col in &table.field_columns {
            col_names.push(quote_ident(&col.name));
            placeholders.push(format!("?{}", col_names.len()));
            let raw = fields.get(&col.name);
            let bound = bind_value(raw, col)?;
            values.push(bound);
        }
        col_names.push(RECORD_ID_COL.into());
        placeholders.push(format!("?{}", col_names.len()));
        values.push(Box::new(record_id.to_string()));

        col_names.push(SCHEDULED_RUN_ID_COL.into());
        placeholders.push(format!("?{}", col_names.len()));
        values.push(Box::new(scheduled_run_id.to_string()));

        col_names.push(EMITTED_AT_COL.into());
        placeholders.push(format!("?{}", col_names.len()));
        values.push(Box::new(emitted_at_ms));

        let sql = format!(
            "INSERT INTO {table} ({cols}) VALUES ({phs})",
            table = quote_ident(&table.name),
            cols = col_names.join(", "),
            phs = placeholders.join(", "),
        );
        let params_iter = values.iter().map(|b| b.as_ref());
        self.tx
            .execute(&sql, params_from_iter(params_iter))
            .map_err(|e| RunError::Output(e.to_string()))?;
        Ok(())
    }

    pub fn commit(self) -> Result<(), RunError> {
        self.tx
            .commit()
            .map_err(|e| RunError::Output(e.to_string()))
    }
}

fn ensure_table(conn: &Connection, t: &TableDef) -> rusqlite::Result<()> {
    let mut cols = Vec::with_capacity(t.field_columns.len() + 3);
    for c in &t.field_columns {
        let null = if c.optional { "" } else { " NOT NULL" };
        cols.push(format!(
            "{} {}{null}",
            quote_ident(&c.name),
            c.storage.sql()
        ));
    }
    cols.push(format!("{RECORD_ID_COL} TEXT NOT NULL"));
    cols.push(format!("{SCHEDULED_RUN_ID_COL} TEXT NOT NULL"));
    cols.push(format!("{EMITTED_AT_COL} INTEGER NOT NULL"));
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        quote_ident(&t.name),
        cols.join(", "),
    );
    conn.execute(&sql, [])?;
    Ok(())
}

fn quote_ident(name: &str) -> String {
    // Use double-quote SQL identifier escape; double any embedded quotes.
    let escaped = name.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn bind_value(raw: Option<&JSONValue>, col: &ColumnDef) -> Result<Box<dyn ToSql>, RunError> {
    // Treat "missing key" and "explicit null" identically — both
    // produce a SQL NULL when the column is optional, or a write
    // error when it isn't. That keeps required-field semantics
    // honest: a recipe that emits the field as `null` shouldn't
    // satisfy a non-optional column any more than omitting it.
    let v = match raw {
        Some(v) if !matches!(v, JSONValue::Null) => v,
        _ => {
            if col.optional {
                return Ok(Box::new(rusqlite::types::Null));
            }
            return Err(RunError::Output(format!(
                "required field '{}' missing or null in emitted record",
                col.name
            )));
        }
    };
    Ok(match col.storage {
        ColumnStorage::Text => match v {
            JSONValue::String(s) => Box::new(s.clone()),
            // Numbers / bools coerce to their string representation so
            // a recipe author can choose to over-declare a column as
            // String without losing data.
            other => Box::new(json_to_text(other)?),
        },
        ColumnStorage::Integer => match v {
            JSONValue::Int(n) => Box::new(*n),
            JSONValue::Bool(b) => Box::new(*b as i64),
            JSONValue::Double(d) => Box::new(*d as i64),
            other => {
                return Err(RunError::Output(format!(
                    "field '{}' is declared Int/Bool but record carried {other:?}",
                    col.name
                )));
            }
        },
        ColumnStorage::Real => match v {
            JSONValue::Double(d) => Box::new(*d),
            JSONValue::Int(n) => Box::new(*n as f64),
            other => {
                return Err(RunError::Output(format!(
                    "field '{}' is declared Double but record carried {other:?}",
                    col.name
                )));
            }
        },
        ColumnStorage::Json => {
            let s = serde_json::to_string(v)?;
            Box::new(s)
        }
    })
}

fn json_to_text(v: &JSONValue) -> Result<String, RunError> {
    Ok(match v {
        JSONValue::Null => String::new(),
        JSONValue::Bool(b) => b.to_string(),
        JSONValue::Int(n) => n.to_string(),
        JSONValue::Double(d) => d.to_string(),
        JSONValue::String(s) => s.clone(),
        JSONValue::Array(_) | JSONValue::Object(_) => serde_json::to_string(v)?,
    })
}

impl From<serde_json::Error> for RunError {
    fn from(e: serde_json::Error) -> Self {
        RunError::Output(format!("json: {e}"))
    }
}

/// Read records back from the output store for a given scheduled-run.
/// Used by the daemon's `load_records` API surface; returns JSON so the
/// caller doesn't need to know the schema.
///
/// The returned rows carry recipe-declared fields only — internal
/// bookkeeping columns (`_scheduled_run_id`, `_emitted_at`) are
/// filtered out so callers see what the recipe actually emitted.
pub fn load_records(
    path: &Path,
    scheduled_run_id: &str,
    type_name: &str,
    limit: u32,
) -> Result<Vec<serde_json::Value>, RunError> {
    // An ephemeral run leaves no file behind, so the persistent path
    // a caller asks about may not exist. Treat that as "no records"
    // rather than a hard error — the caller's mental model is "show me
    // what this scheduled-run produced," and a missing file faithfully
    // represents zero persisted records.
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(path).map_err(|e| RunError::Output(e.to_string()))?;
    // Discover the column list dynamically — the daemon's `Run` record
    // doesn't carry the recipe-derived schema separately.
    let all_cols = pragma_columns(&conn, type_name)?;
    if all_cols.is_empty() {
        return Ok(Vec::new());
    }
    let cols: Vec<String> = all_cols
        .into_iter()
        .filter(|c| c != SCHEDULED_RUN_ID_COL && c != EMITTED_AT_COL)
        .collect();
    if cols.is_empty() {
        return Ok(Vec::new());
    }
    let select_cols = cols
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select_cols} FROM {table} WHERE {SCHEDULED_RUN_ID_COL} = ?1 ORDER BY {EMITTED_AT_COL} ASC LIMIT ?2",
        table = quote_ident(type_name),
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| RunError::Output(e.to_string()))?;
    let mut out = Vec::new();
    let rows = stmt
        .query_map(rusqlite::params![scheduled_run_id, limit], |r| {
            let mut obj = serde_json::Map::new();
            for (i, name) in cols.iter().enumerate() {
                let value = sqlite_value_to_json(r, i)?;
                obj.insert(name.clone(), value);
            }
            Ok(serde_json::Value::Object(obj))
        })
        .map_err(|e| RunError::Output(e.to_string()))?;
    for row in rows {
        out.push(row.map_err(|e| RunError::Output(e.to_string()))?);
    }
    Ok(out)
}

fn pragma_columns(conn: &Connection, table: &str) -> Result<Vec<String>, RunError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", quote_ident(table)))
        .map_err(|e| RunError::Output(e.to_string()))?;
    let mut out = Vec::new();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| RunError::Output(e.to_string()))?;
    for row in rows {
        out.push(row.map_err(|e| RunError::Output(e.to_string()))?);
    }
    Ok(out)
}

fn sqlite_value_to_json(r: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<serde_json::Value> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(idx)? {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(n) => serde_json::Value::from(n),
        // `Number::from_f64` returns `None` for NaN / Inf — JSON has
        // no representation for them, so we surface them as `null`
        // rather than failing the whole row. Recipe authors should
        // not be emitting non-finite floats; if one slips through, the
        // null is the deliberate downgrade.
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(t) => match std::str::from_utf8(t) {
            Ok(s) => serde_json::Value::String(s.to_string()),
            Err(_) => serde_json::Value::Null,
        },
        ValueRef::Blob(b) => serde_json::Value::String(format!("<blob:{} bytes>", b.len())),
    })
}
