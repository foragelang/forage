//! Output-schema derivation: every `emit Foo` referenced in the
//! recipe contributes one `TableDef`, and the recipe-declared field
//! types lower into the documented SQL storage classes.
//!
//! Then an end-to-end check: open an `OutputStore` against a
//! tempfile, write a record, verify the row lives at the right
//! columns.

use forage_core::{LinkedRecipe, link_standalone, parse};
use forage_daemon::{ColumnStorage, OutputStore, derive_schema};
use indexmap::IndexMap;
use rusqlite::Connection;

const RECIPE: &str = r#"recipe "products"
engine http

type Product {
    id: String
    price: Double
    available: Bool
    stock_units: Int
    tags: [String]
}

step list {
    method "GET"
    url    "https://example.test/products"
}

for $p in $list.products[*] {
    emit Product {
        id ← $p.id,
        price ← $p.price,
        available ← $p.available,
        stock_units ← $p.stock_units,
        tags ← $p.tags
    }
}
"#;

fn module_for(source: &str) -> forage_core::LinkedModule {
    let parsed = parse(source).expect("parse");
    let outcome = link_standalone(parsed);
    assert!(
        !outcome.report.has_errors(),
        "link errors: {:?}",
        outcome.report.issues,
    );
    outcome.module.expect("linker produces module")
}

#[test]
fn derive_schema_emits_one_table_per_emit_type() {
    let module = module_for(RECIPE);
    let tables = derive_schema(&module);
    assert_eq!(tables.len(), 1);
    let t = &tables[0];
    assert_eq!(t.name, "Product");
    let cols: Vec<(&str, ColumnStorage)> = t
        .field_columns
        .iter()
        .map(|c| (c.name.as_str(), c.storage))
        .collect();
    assert_eq!(
        cols,
        vec![
            ("id", ColumnStorage::Text),
            ("price", ColumnStorage::Real),
            ("available", ColumnStorage::Integer),
            ("stock_units", ColumnStorage::Integer),
            ("tags", ColumnStorage::Json),
        ]
    );
}

#[test]
fn output_store_creates_table_with_metadata_columns() {
    let module = module_for(RECIPE);
    let tables = derive_schema(&module);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("products.sqlite");
    let _store = OutputStore::open(&path, tables).expect("open output store");

    let conn = Connection::open(&path).unwrap();
    let info: Vec<(String, String)> = conn
        .prepare(r#"PRAGMA table_info("Product")"#)
        .unwrap()
        .query_map([], |r| {
            let name: String = r.get(1)?;
            let ty: String = r.get(2)?;
            Ok((name, ty))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let names: Vec<&str> = info.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"id"));
    assert!(names.contains(&"price"));
    assert!(names.contains(&"available"));
    assert!(names.contains(&"stock_units"));
    assert!(names.contains(&"tags"));
    assert!(names.contains(&"_id"));
    assert!(names.contains(&"_scheduled_run_id"));
    assert!(names.contains(&"_emitted_at"));

    // SQL storage classes correspond to the column type mapping.
    let by_name: std::collections::HashMap<&str, &str> =
        info.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    assert_eq!(by_name["id"], "TEXT");
    assert_eq!(by_name["price"], "REAL");
    assert_eq!(by_name["available"], "INTEGER");
    assert_eq!(by_name["stock_units"], "INTEGER");
    assert_eq!(by_name["tags"], "TEXT"); // JSON columns store as TEXT
    assert_eq!(by_name["_id"], "TEXT");
    assert_eq!(by_name["_scheduled_run_id"], "TEXT");
    assert_eq!(by_name["_emitted_at"], "INTEGER");
}

#[test]
fn write_record_round_trips_through_load_records() {
    let module = module_for(RECIPE);
    let tables = derive_schema(&module);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("products.sqlite");
    let mut store = OutputStore::open(&path, tables).expect("open");

    let mut fields: IndexMap<String, forage_core::ast::JSONValue> = IndexMap::new();
    fields.insert(
        "id".into(),
        forage_core::ast::JSONValue::String("sku-1".into()),
    );
    fields.insert("price".into(), forage_core::ast::JSONValue::Double(9.95));
    fields.insert("available".into(), forage_core::ast::JSONValue::Bool(true));
    fields.insert("stock_units".into(), forage_core::ast::JSONValue::Int(42));
    fields.insert(
        "tags".into(),
        forage_core::ast::JSONValue::Array(vec![
            forage_core::ast::JSONValue::String("featured".into()),
            forage_core::ast::JSONValue::String("flash".into()),
        ]),
    );

    let mut tx = store.begin_tx().unwrap();
    tx.write_record("sched-1", 1_700_000_000_000, "rec-0", "Product", &fields)
        .expect("write");
    tx.commit().expect("commit");

    let records = forage_daemon::load_records(&path, "sched-1", "Product", 10).expect("load");
    assert_eq!(records.len(), 1);
    let obj = records[0].as_object().unwrap();
    assert_eq!(obj["id"], serde_json::json!("sku-1"));
    assert_eq!(obj["price"], serde_json::json!(9.95));
    assert_eq!(obj["available"], serde_json::json!(1));
    assert_eq!(obj["stock_units"], serde_json::json!(42));
    assert_eq!(obj["_id"], serde_json::json!("rec-0"));
    // Tags round-trip as the JSON-encoded array text (it's stored in
    // a TEXT column with the `Json` storage tag).
    let tags_text = obj["tags"].as_str().expect("tags is text-encoded JSON");
    let tags: serde_json::Value = serde_json::from_str(tags_text).unwrap();
    assert_eq!(tags, serde_json::json!(["featured", "flash"]));
    // Bookkeeping columns are filtered out at the load boundary — the
    // UI tables show recipe-declared fields + `_id` (the synthetic
    // record identity that `Ref<T>` values point at), not the audit
    // metadata.
    assert!(!obj.contains_key("_scheduled_run_id"));
    assert!(!obj.contains_key("_emitted_at"));
}

#[test]
fn load_records_excludes_bookkeeping_columns() {
    let module = module_for(RECIPE);
    let tables = derive_schema(&module);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("products.sqlite");
    let mut store = OutputStore::open(&path, tables).expect("open");

    let mut fields: IndexMap<String, forage_core::ast::JSONValue> = IndexMap::new();
    fields.insert(
        "id".into(),
        forage_core::ast::JSONValue::String("sku-1".into()),
    );
    fields.insert("price".into(), forage_core::ast::JSONValue::Double(9.95));
    fields.insert("available".into(), forage_core::ast::JSONValue::Bool(true));
    fields.insert("stock_units".into(), forage_core::ast::JSONValue::Int(1));
    fields.insert("tags".into(), forage_core::ast::JSONValue::Array(vec![]));

    let mut tx = store.begin_tx().unwrap();
    tx.write_record("sched-1", 1_700_000_000_000, "rec-0", "Product", &fields)
        .expect("write");
    tx.commit().expect("commit");

    let rows = forage_daemon::load_records(&path, "sched-1", "Product", 10).expect("load");
    let row = rows[0].as_object().unwrap();
    let keys: std::collections::HashSet<&str> = row.keys().map(|s| s.as_str()).collect();
    assert!(keys.contains("id"));
    assert!(keys.contains("price"));
    assert!(keys.contains("available"));
    assert!(keys.contains("stock_units"));
    assert!(keys.contains("tags"));
    assert!(keys.contains("_id"));
    assert!(!keys.contains("_scheduled_run_id"));
    assert!(!keys.contains("_emitted_at"));
}

/// `derive_schema` against a composition recipe with a declared
/// `emits` clause pre-creates the tables the chain's output store
/// needs. The composition body has no `emit` statements of its own —
/// records arrive via the chain's terminal stage — so derive_schema
/// reads the declared `emits` to know which types the store must hold.
#[test]
fn derive_schema_creates_tables_from_emits_on_composition_body() {
    const COMPOSITION: &str = r#"recipe "composed"
engine http

type Product {
    id: String
}

emits Product

compose "scrape" | "enrich"
"#;
    // Build the module manually: this test pins `derive_schema`'s
    // behavior on a composition root with declared emits, regardless
    // of whether the chain's stages resolve (the linker's tests cover
    // the validation side).
    let parsed = parse(COMPOSITION).expect("parse");
    let catalog: forage_core::TypeCatalog = forage_core::TypeCatalog::from_file(&parsed);
    let module = forage_core::LinkedModule {
        root: LinkedRecipe::from_file(parsed),
        stages: std::collections::BTreeMap::new(),
        catalog: forage_core::SerializableCatalog::from(catalog),
    };
    let tables = derive_schema(&module);
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Product"],
        "declared `emits Product` on a composition recipe must contribute a Product table"
    );
}

/// `derive_schema` against a composition recipe without a declared
/// `emits` clause chases the chain to its terminal stage. Tests pin
/// the resolved-terminal recursion: `composed = compose scrape |
/// enrich`, both stages emit `Product`, the composition declares no
/// `emits` of its own, but `derive_schema` still produces a `Product`
/// table because the terminal stage (`enrich`) declares emits via the
/// linked closure.
#[test]
fn derive_schema_chases_terminal_stage_emits_for_composition_without_declared_emits() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(
        root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Product.forage"),
        "share type Product { id: String }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("scrape.forage"),
        "recipe \"scrape\"\nengine http\n\
         emits Product\n\
         step list { method \"GET\" url \"https://x.test\" }\n\
         emit Product { id ← \"a\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("enrich.forage"),
        "recipe \"enrich\"\nengine http\n\
         input prior: [Product]\n\
         emits Product\n\
         for $p in $input.prior { emit Product { id ← $p.id } }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("composed.forage"),
        "recipe \"composed\"\nengine http\ncompose \"scrape\" | \"enrich\"\n",
    )
    .unwrap();
    let workspace = forage_core::load(root).unwrap();
    let outcome = forage_core::link(&workspace, "composed").expect("link");
    assert!(
        !outcome.report.has_errors(),
        "link errors: {:?}",
        outcome.report.issues,
    );
    let module = outcome.module.expect("linker produces module");
    let tables = derive_schema(&module);
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Product"],
        "terminal stage's emits drive derive_schema when the composition omits its own clause"
    );
}
