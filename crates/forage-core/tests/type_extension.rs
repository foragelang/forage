//! End-to-end tests for `type Child extends Parent@vN` across the
//! parse, validate, and workspace-catalog surfaces.
//!
//! The adapter-recipe test exercises the typed-hub composition flow:
//! a `parent_emitter | adapter_to_child` chain where the adapter
//! declares `input Parent → emits Child`, the validator confirms the
//! pipe boundary, and the type catalog resolves the child's effective
//! shape (parent fields + alignments + child additions).

use std::fs;
use std::path::Path;

use forage_core::ast::{FieldType, ForageFile};
use forage_core::parse::parse;
use forage_core::validate::{validate, ValidationCode};
use forage_core::workspace::{
    load, type_cache_file, RecipeSignature, RecipeSignatures, TypeCatalog, LOCKFILE_NAME,
    MANIFEST_NAME,
};

const STARTER_MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn signatures_from(files: &[(&str, &ForageFile)]) -> RecipeSignatures {
    let mut sigs = RecipeSignatures::default();
    for (name, f) in files {
        sigs.insert((*name).to_string(), RecipeSignature::from_file(f));
    }
    sigs
}

#[test]
fn workspace_local_extension_resolves_through_share() {
    // A `share`d Parent in one workspace file is the catalog entry
    // that a `Child extends Parent@v1` in another file refers to. The
    // effective lookup carries Parent's fields + alignments into
    // Child.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
    write(
        &root.join("parent.forage"),
        "share type Parent\n    aligns schema.org/JobPosting\n{\n    id: String\n    title: String aligns schema.org/title\n}\n",
    );
    write(
        &root.join("child.forage"),
        "share type Child extends Parent@v1\n    aligns wikidata/Q1056\n{\n    salaryMin: Int?\n    salaryMax: Int?\n}\n",
    );
    let recipe_path = root.join("rec.forage");
    write(&recipe_path, "recipe \"rec\"\nengine http\n");

    let ws = load(root).unwrap();
    let cat = ws.catalog_from_disk(&recipe_path).unwrap();

    let child = cat.lookup("Child").expect("child resolves");
    let names: Vec<&str> = child.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["id", "title", "salaryMin", "salaryMax"]);
    let title = child.fields.iter().find(|f| f.name == "title").unwrap();
    assert_eq!(
        title.alignment.as_ref().unwrap().term,
        "title",
        "inherited field-level alignment carries through",
    );
    let mut alignments: Vec<String> = child
        .alignments
        .iter()
        .map(|a| format!("{}/{}", a.ontology, a.term))
        .collect();
    alignments.sort();
    assert_eq!(
        alignments,
        vec!["schema.org/JobPosting", "wikidata/Q1056"],
        "type-level alignments merge parent + child",
    );
}

#[test]
fn workspace_local_extension_validates_clean_end_to_end() {
    // The whole pipeline — parse, workspace catalog, validate — runs
    // clean against a parent/child pair declared across two files.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
    write(
        &root.join("decls.forage"),
        "share type Parent { id: String, name: String }\nshare type Child extends Parent@v1 { extra: String }\n",
    );
    let recipe_path = root.join("rec.forage");
    write(
        &recipe_path,
        "recipe \"rec\"\nengine http\nemits Child\nstep list { method \"GET\" url \"https://x.test\" }\nfor $r in $list[*] {\n    emit Child { id \u{2190} $r.id, name \u{2190} $r.name, extra \u{2190} $r.extra }\n}\n",
    );
    let ws = load(root).unwrap();
    let cat = ws.catalog_from_disk(&recipe_path).unwrap();
    let recipe_src = fs::read_to_string(&recipe_path).unwrap();
    let recipe = parse(&recipe_src).unwrap();
    let rep = validate(&recipe, &cat, &RecipeSignatures::default());
    assert!(
        !rep.has_errors(),
        "child-emit using inherited fields must validate: {:?}",
        rep.issues,
    );
}

#[test]
#[serial_test::serial]
fn hub_dep_extension_resolves_through_lockfile_cached_parent() {
    // The hub-dep path: the parent's source lives in the workspace's
    // hub cache directory, pinned by the lockfile. The validator
    // resolves `extends @upstream/JobPosting@v1` against the same
    // catalog that workspace-shared types feed into.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("hub-cache");
    let cache_path = type_cache_file(&cache, "upstream", "JobPosting", 1);
    write(
        &cache_path,
        "share type JobPosting\n    aligns schema.org/JobPosting\n{\n    id: String\n    title: String\n}\n",
    );

    let ws_root = tmp.path().join("ws");
    write(
        &ws_root.join(MANIFEST_NAME),
        "name = \"alice/enhanced\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
    );
    write(
        &ws_root.join(LOCKFILE_NAME),
        "[types.\"upstream/JobPosting\"]\nversion = 1\nhash = \"\"\n",
    );
    let recipe_path = ws_root.join("rec.forage");
    write(
        &recipe_path,
        "share type EnhancedJobPosting extends @upstream/JobPosting@v1 {\n    salaryMin: Int?\n}\nrecipe \"rec\"\nengine http\nemits EnhancedJobPosting\nstep list { method \"GET\" url \"https://x.test\" }\nfor $r in $list[*] {\n    emit EnhancedJobPosting { id \u{2190} $r.id, title \u{2190} $r.title, salaryMin \u{2190} $r.salary }\n}\n",
    );

    let prev = std::env::var("FORAGE_HUB_CACHE").ok();
    // SAFETY: env mutation is unsafe in Rust 2024; this test runs
    // against the process-global env. It restores the prior value
    // before returning.
    unsafe {
        std::env::set_var("FORAGE_HUB_CACHE", &cache);
    }

    let result = (|| {
        let ws = load(&ws_root)?;
        let cat = ws.catalog_from_disk(&recipe_path)?;
        let recipe_src = fs::read_to_string(&recipe_path).unwrap();
        let recipe = parse(&recipe_src).unwrap();
        let rep = validate(&recipe, &cat, &RecipeSignatures::default());
        Ok::<_, forage_core::workspace::WorkspaceError>((cat, rep))
    })();

    // SAFETY: see above.
    match prev {
        Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
        None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
    }

    let (cat, rep) = result.expect("workspace load + validate");
    assert!(
        !rep.has_errors(),
        "hub-cached parent must validate: {:?}",
        rep.issues,
    );
    let enhanced = cat.lookup("EnhancedJobPosting").expect("effective child");
    let names: Vec<&str> = enhanced.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["id", "title", "salaryMin"]);
    assert!(
        enhanced
            .alignments
            .iter()
            .any(|a| a.ontology == "schema.org" && a.term == "JobPosting"),
        "parent's alignment carries through",
    );
}

#[test]
#[serial_test::serial]
fn extension_chain_with_missing_link_surfaces_unknown_extended_type() {
    // The lockfile pin's cache file is absent — the catalog can't
    // materialise the parent, the validator must flag
    // `UnknownExtendedType` so the recipe author lands on the
    // missing dep.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("hub-cache");
    // Intentionally do NOT write the parent's cache file.
    fs::create_dir_all(&cache).unwrap();

    let ws_root = tmp.path().join("ws");
    write(
        &ws_root.join(MANIFEST_NAME),
        "name = \"alice/enhanced\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
    );
    write(
        &ws_root.join(LOCKFILE_NAME),
        "[types.\"upstream/JobPosting\"]\nversion = 1\nhash = \"\"\n",
    );
    let recipe_path = ws_root.join("rec.forage");
    write(
        &recipe_path,
        "share type Ext extends @upstream/JobPosting@v1 {\n    extra: String\n}\nrecipe \"rec\"\nengine http\n",
    );

    let prev = std::env::var("FORAGE_HUB_CACHE").ok();
    // SAFETY: env mutation is unsafe in Rust 2024.
    unsafe {
        std::env::set_var("FORAGE_HUB_CACHE", &cache);
    }

    let ws = load(&ws_root).unwrap();
    let cat = ws.catalog_from_disk(&recipe_path).unwrap();
    let recipe_src = fs::read_to_string(&recipe_path).unwrap();
    let recipe = parse(&recipe_src).unwrap();
    let rep = validate(&recipe, &cat, &RecipeSignatures::default());

    // SAFETY: see above.
    match prev {
        Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
        None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
    }

    assert!(
        rep.errors()
            .any(|i| i.code == ValidationCode::UnknownExtendedType),
        "missing hub-cached parent must surface UnknownExtendedType: {:?}",
        rep.issues,
    );
}

#[test]
fn adapter_recipe_pipes_parent_records_into_child() {
    // The adapter-recipe pattern: a recipe whose input is the parent
    // type and whose output is the child type. The validator
    // confirms the pipe boundary and the type catalog confirms the
    // child carries the parent's fields. A producer recipe emitting
    // Parent records pipes cleanly into the adapter.
    let producer_src = "\
        recipe \"producer\"\n\
        engine http\n\
        share type Parent { id: String, title: String }\n\
        emits Parent\n\
        step list { method \"GET\" url \"https://x.test\" }\n\
        emit Parent { id \u{2190} \"p1\", title \u{2190} \"Engineer\" }\n";
    let adapter_src = "\
        recipe \"to-enhanced\"\n\
        engine http\n\
        share type Parent { id: String, title: String }\n\
        share type Enhanced extends Parent@v1 {\n\
            salaryMin: Int?\n\
        }\n\
        input parents: [Parent]\n\
        emits Enhanced\n\
        for $p in $input.parents[*] {\n\
            emit Enhanced { id \u{2190} $p.id, title \u{2190} $p.title, salaryMin \u{2190} null }\n\
        }\n";
    let chain_src = "\
        recipe \"chain\"\n\
        engine http\n\
        share type Parent { id: String, title: String }\n\
        share type Enhanced extends Parent@v1 {\n\
            salaryMin: Int?\n\
        }\n\
        emits Enhanced\n\
        compose \"producer\" | \"to-enhanced\"\n";

    let producer = parse(producer_src).expect("producer parses");
    let adapter = parse(adapter_src).expect("adapter parses");
    let chain = parse(chain_src).expect("chain parses");

    // Adapter on its own: parent fields visible to `emit Enhanced`
    // even though Enhanced only declares the extra field.
    let cat = TypeCatalog::from_file(&adapter);
    let rep = validate(&adapter, &cat, &RecipeSignatures::default());
    assert!(
        !rep.has_errors(),
        "adapter recipe must validate (parent fields inherited): {:?}",
        rep.issues,
    );

    // Chain: `producer | to-enhanced` type-checks because the adapter
    // has `input parents: [Parent]` matching the producer's emitted
    // `Parent` (declared via the producer's `emits Parent` clause).
    let cat = TypeCatalog::from_file(&chain);
    let signatures = signatures_from(&[("producer", &producer), ("to-enhanced", &adapter)]);
    let rep = validate(&chain, &cat, &signatures);
    assert!(
        !rep.has_errors(),
        "adapter pipes cleanly from a producer of the parent type: {:?}",
        rep.issues,
    );
}

#[test]
fn adapter_recipe_rejected_when_emits_unrelated_to_input() {
    // The validator's IncompatiblePipeStage rule rejects an adapter
    // whose emitted type doesn't match the upstream output: there's
    // no structural link between the input and emitted types.
    let producer_src = "\
        recipe \"producer\"\n\
        engine http\n\
        share type Parent { id: String }\n\
        emits Parent\n\
        step list { method \"GET\" url \"https://x.test\" }\n\
        emit Parent { id \u{2190} \"p1\" }\n";
    let unrelated_src = "\
        recipe \"unrelated\"\n\
        engine http\n\
        share type Apple { id: String }\n\
        share type Banana { id: String }\n\
        input apples: [Apple]\n\
        emits Banana\n\
        for $a in $input.apples[*] {\n\
            emit Banana { id \u{2190} $a.id }\n\
        }\n";
    let chain_src = "\
        recipe \"chain\"\n\
        engine http\n\
        share type Parent { id: String }\n\
        share type Banana { id: String }\n\
        emits Banana\n\
        compose \"producer\" | \"unrelated\"\n";

    let producer = parse(producer_src).unwrap();
    let unrelated = parse(unrelated_src).unwrap();
    let chain = parse(chain_src).unwrap();

    let cat = TypeCatalog::from_file(&chain);
    let signatures = signatures_from(&[("producer", &producer), ("unrelated", &unrelated)]);
    let rep = validate(&chain, &cat, &signatures);
    assert!(
        rep.errors()
            .any(|i| i.code == ValidationCode::IncompatiblePipeStage),
        "unrelated downstream input type must surface IncompatiblePipeStage: {:?}",
        rep.issues,
    );
}

#[test]
fn child_can_redeclare_parent_field_with_different_alignment() {
    // Override semantics: a child redeclaring `name: String aligns
    // wikidata/P2561` swaps the parent's `aligns schema.org/name`
    // without becoming an IncompatibleExtension.
    let src = "\
        share type Parent {\n\
            id:   String\n\
            name: String aligns schema.org/name\n\
        }\n\
        share type Child extends Parent@v1 {\n\
            name: String aligns wikidata/P2561\n\
            extra: String\n\
        }\n";
    let r = parse(src).unwrap();
    let cat = TypeCatalog::from_file(&r);
    let rep = validate(&r, &cat, &RecipeSignatures::default());
    assert!(!rep.has_errors(), "override is permitted: {:?}", rep.issues);

    let child = cat.lookup("Child").unwrap();
    let name = child.fields.iter().find(|f| f.name == "name").unwrap();
    let align = name.alignment.as_ref().unwrap();
    assert_eq!(align.ontology, "wikidata");
    assert_eq!(align.term, "P2561");
}

#[test]
fn child_can_redeclare_parent_field_without_alignment_to_drop_inheritance() {
    // The propagation policy: a child redeclaration without `aligns`
    // drops the parent's field-level alignment. The catalog must
    // reflect that explicitly rather than silently keeping the
    // parent's URI.
    let src = "\
        share type Parent {\n\
            id:   String\n\
            name: String aligns schema.org/name\n\
        }\n\
        share type Child extends Parent@v1 {\n\
            name: String\n\
            extra: String\n\
        }\n";
    let r = parse(src).unwrap();
    let cat = TypeCatalog::from_file(&r);
    let rep = validate(&r, &cat, &RecipeSignatures::default());
    assert!(!rep.has_errors(), "override drop is permitted: {:?}", rep.issues);

    let child = cat.lookup("Child").unwrap();
    let name = child.fields.iter().find(|f| f.name == "name").unwrap();
    assert!(
        name.alignment.is_none(),
        "child redeclaration without `aligns` drops the parent's alignment",
    );
    // Sanity: the field type and optionality still match the parent.
    assert!(matches!(name.ty, FieldType::String));
    assert!(!name.optional);
}
