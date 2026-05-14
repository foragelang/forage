//! Output-schema derivation: every `emit Foo` referenced in the
//! recipe contributes one `TableDef`, and the recipe-declared field
//! types lower into the documented SQL storage classes.
//!
//! Then an end-to-end check: open an `OutputStore` against a
//! tempfile, write a record, verify the row lives at the right
//! columns.

use forage_core::TypeCatalog;
use forage_core::parse;
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

#[test]
fn derive_schema_emits_one_table_per_emit_type() {
    let recipe = parse(RECIPE).expect("parse");
    let catalog = TypeCatalog::from_recipe(&recipe);
    let tables = derive_schema(&recipe, &catalog);
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
    let recipe = parse(RECIPE).expect("parse");
    let catalog = TypeCatalog::from_recipe(&recipe);
    let tables = derive_schema(&recipe, &catalog);
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
    assert_eq!(by_name["_scheduled_run_id"], "TEXT");
    assert_eq!(by_name["_emitted_at"], "INTEGER");
}

#[test]
fn write_record_round_trips_through_load_records() {
    let recipe = parse(RECIPE).expect("parse");
    let catalog = TypeCatalog::from_recipe(&recipe);
    let tables = derive_schema(&recipe, &catalog);
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
    fields.insert(
        "stock_units".into(),
        forage_core::ast::JSONValue::Int(42),
    );
    fields.insert(
        "tags".into(),
        forage_core::ast::JSONValue::Array(vec![
            forage_core::ast::JSONValue::String("featured".into()),
            forage_core::ast::JSONValue::String("flash".into()),
        ]),
    );

    let mut tx = store.begin_tx().unwrap();
    tx.write_record("sched-1", 1_700_000_000_000, "Product", &fields)
        .expect("write");
    tx.commit().expect("commit");

    let records =
        forage_daemon::load_records(&path, "sched-1", "Product", 10).expect("load");
    assert_eq!(records.len(), 1);
    let obj = records[0].as_object().unwrap();
    assert_eq!(obj["id"], serde_json::json!("sku-1"));
    assert_eq!(obj["price"], serde_json::json!(9.95));
    assert_eq!(obj["available"], serde_json::json!(1));
    assert_eq!(obj["stock_units"], serde_json::json!(42));
    // Tags round-trip as the JSON-encoded array text (it's stored in
    // a TEXT column with the `Json` storage tag).
    let tags_text = obj["tags"].as_str().expect("tags is text-encoded JSON");
    let tags: serde_json::Value = serde_json::from_str(tags_text).unwrap();
    assert_eq!(tags, serde_json::json!(["featured", "flash"]));
    assert_eq!(obj["_scheduled_run_id"], serde_json::json!("sched-1"));
    assert_eq!(obj["_emitted_at"], serde_json::json!(1_700_000_000_000_i64));
}
