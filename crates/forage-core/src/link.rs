//! Validation + closure construction in one pass.
//!
//! Where [`validate`] checks a single parsed file in isolation,
//! [`link`] resolves every composition-stage reference against the
//! workspace, recursively validates each linked stage, and produces
//! a [`LinkedModule`] — the closure the runtime consumes to execute
//! the recipe without doing any more name resolution.
//!
//! The single-file [`validate`] entry point survives as the
//! per-recipe checker `link` calls under the hood. Callers that
//! need a runnable artifact (CLI run / daemon deploy / Studio's
//! "save snapshot") go through `link`; callers that just want
//! diagnostics for an in-flight edit (LSP, Studio's per-keystroke
//! validation) still call `validate` directly.
//!
//! Lonely-recipe mode (`link_standalone`) is the path for files
//! outside any workspace — composition stages then have nothing to
//! resolve against and surface as `UnknownComposeStage` diagnostics,
//! same as today.

use std::collections::BTreeMap;

use crate::ast::{ForageFile, RecipeBody, RecipeRef};
use crate::linked::{LinkedModule, LinkedRecipe};
use crate::validate::{ValidationReport, validate};
use crate::workspace::{
    RecipeSignatures, SerializableCatalog, TypeCatalog, Workspace, WorkspaceError,
};

/// Outcome of a link attempt. `module` is populated when validation
/// of the root and every transitively-reachable stage clears; on any
/// validation failure the report carries the issues and `module` is
/// `None`.
#[derive(Debug)]
pub struct LinkOutcome {
    pub module: Option<LinkedModule>,
    pub report: ValidationReport,
}

/// Hard failures that prevent linking from producing even a diagnostic
/// report — workspace I/O errors, lockfile corruption, type-cache
/// reads. Recipe-level validation issues (unknown stages, cycles,
/// emit/input mismatches) are carried inside [`LinkOutcome::report`]
/// instead.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("workspace: {0}")]
    Workspace(#[from] WorkspaceError),
    #[error("recipe '{0}' is not in the workspace")]
    UnknownRecipe(String),
    #[error("recipe '{0}' has no header — link target must be a recipe-bearing file")]
    HeaderlessRecipe(String),
}

/// Link a recipe in the context of a workspace. Resolves the root's
/// composition stages (when present) against `workspace.recipe_by_name`,
/// recursively links each reachable stage, and builds a unified type
/// catalog covering the root plus every stage. The closure that comes
/// back is what the runtime consumes; the diagnostic report mirrors
/// what a per-recipe `validate` pass on each linked node would have
/// produced.
pub fn link(workspace: &Workspace, recipe_name: &str) -> Result<LinkOutcome, LinkError> {
    let recipe = workspace
        .recipe_by_name(recipe_name)
        .ok_or_else(|| LinkError::UnknownRecipe(recipe_name.to_string()))?;
    let root_file = recipe.file.clone();

    let catalog = workspace.catalog(&root_file, |p| std::fs::read_to_string(p))?;
    let signatures = workspace.recipe_signatures();

    let mut issues = Vec::new();
    let root_report = validate(&root_file, &catalog, &signatures);
    issues.extend(root_report.issues);

    // Resolve stage closure. Hub-dep stages, unknown stages, and
    // cycles are all already covered by the per-recipe validator's
    // `check_composition` pass; here we only walk the stages we can
    // resolve, and the per-stage validate pass surfaces issues on
    // each. The plan's `MultiTypeComposeStage` / `EmptyComposeStage`
    // diagnostics live on the *root* recipe's report (anchored on its
    // stage span), so the per-stage pass never re-surfaces them.
    let stages = if matches!(root_file.body, RecipeBody::Composition(_)) {
        collect_stage_closure(workspace, &root_file)?
    } else {
        BTreeMap::new()
    };

    // Validate each linked stage in its own right. Stages can fail
    // their own per-recipe checks (e.g. unknown types in an inferred
    // emit, broken auth strategy) and those diagnostics ride back on
    // the same report so the user sees the full picture from the
    // root's link attempt.
    for stage in stages.values() {
        let stage_report = validate(&stage.file, &catalog, &signatures);
        issues.extend(stage_report.issues);
    }

    let report = ValidationReport { issues };
    let module = if report.has_errors() {
        None
    } else {
        Some(LinkedModule {
            root: LinkedRecipe::from_file(root_file),
            stages,
            catalog: SerializableCatalog::from(catalog),
        })
    };

    Ok(LinkOutcome { module, report })
}

/// Link a `.forage` file outside any workspace. Stages have nothing
/// to resolve against, so a composition body whose stages reference
/// peer recipes surfaces as `UnknownComposeStage` diagnostics — same
/// shape as today's workspaceless validation, just expressed through
/// the linker.
pub fn link_standalone(file: ForageFile) -> LinkOutcome {
    let catalog = TypeCatalog::from_file(&file);
    let signatures = RecipeSignatures::default();
    let report = validate(&file, &catalog, &signatures);
    let module = if report.has_errors() {
        None
    } else {
        Some(LinkedModule {
            root: LinkedRecipe::from_file(file),
            stages: BTreeMap::new(),
            catalog: SerializableCatalog::from(catalog),
        })
    };
    LinkOutcome { module, report }
}

/// Walk every reachable composition stage from `root` and collect the
/// linked-recipe peers into a name-keyed map. Stops at hub-dep stages
/// (`author.is_some()`) and at names that don't resolve in the
/// workspace — both surface as validation diagnostics on the root's
/// own report, so the closure walker just skips them silently.
///
/// Cycles are broken by the `visited` check: revisiting an already-
/// linked name short-circuits without re-walking, leaving the
/// validator's `ComposeCycle` rule (on the root's report) as the
/// authoritative diagnostic.
fn collect_stage_closure(
    workspace: &Workspace,
    root: &ForageFile,
) -> Result<BTreeMap<String, LinkedRecipe>, LinkError> {
    let mut out: BTreeMap<String, LinkedRecipe> = BTreeMap::new();
    if let Some(comp) = root.body.composition() {
        for stage in &comp.stages {
            walk_stage(workspace, stage, &mut out)?;
        }
    }
    Ok(out)
}

fn walk_stage(
    workspace: &Workspace,
    stage: &RecipeRef,
    out: &mut BTreeMap<String, LinkedRecipe>,
) -> Result<(), LinkError> {
    if stage.author.is_some() {
        // Hub-dep stages — `HubDepStageUnsupported` on the validator.
        return Ok(());
    }
    if out.contains_key(&stage.name) {
        return Ok(());
    }
    let Some(peer) = workspace.recipe_by_name(&stage.name) else {
        // `UnknownComposeStage` on the validator.
        return Ok(());
    };
    let peer_file = peer.file.clone();
    let linked = LinkedRecipe::from_file(peer_file.clone());
    out.insert(stage.name.clone(), linked);
    if let Some(comp) = peer_file.body.composition() {
        for inner in &comp.stages {
            walk_stage(workspace, inner, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::load;
    use std::path::Path;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    const STARTER_MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

    /// Linking a scraping recipe with no composition body produces a
    /// module with the recipe as root and an empty stage map.
    #[test]
    fn link_scraping_recipe_has_no_stages() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("scrape.forage"),
            "recipe \"scrape\"\nengine http\n\
             share type Product { id: String }\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "scrape").expect("link succeeds");
        assert!(!outcome.report.has_errors(), "report: {:?}", outcome.report);
        let module = outcome.module.expect("module produced");
        assert!(module.stages.is_empty());
        assert_eq!(module.root.file.recipe_name(), Some("scrape"));
        assert_eq!(
            module.root.emit_types.iter().cloned().collect::<Vec<_>>(),
            vec!["Product".to_string()],
        );
    }

    /// Two-stage `compose A | B` resolves both stages into the
    /// module's stage map; the catalog merges types contributed by
    /// each file.
    #[test]
    fn link_two_stage_composition() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("scrape.forage"),
            "recipe \"scrape\"\nengine http\n\
             share type Product { id: String }\n\
             emits Product\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("enrich.forage"),
            "recipe \"enrich\"\nengine http\n\
             share type Product { id: String }\n\
             input prior: [Product]\n\
             emits Product\n\
             for $p in $input.prior {\n\
                 emit Product { id ← $p.id }\n\
             }\n",
        );
        write(
            &root.join("composed.forage"),
            "recipe \"composed\"\nengine http\n\
             compose \"scrape\" | \"enrich\"\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "composed").expect("link succeeds");
        assert!(
            !outcome.report.has_errors(),
            "errors: {:?}",
            outcome
                .report
                .issues
                .iter()
                .map(|i| (i.code, &i.message))
                .collect::<Vec<_>>(),
        );
        let module = outcome.module.expect("module produced");
        assert_eq!(module.stages.len(), 2);
        assert!(module.stages.contains_key("scrape"));
        assert!(module.stages.contains_key("enrich"));
    }

    /// A multi-level composition (`outer = compose middle | tail`;
    /// `middle = compose leaf | passthrough`) flattens into one
    /// stage map keyed by recipe name.
    #[test]
    fn link_multi_level_composition_flattens_closure() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("leaf.forage"),
            "recipe \"leaf\"\nengine http\n\
             share type Product { id: String }\n\
             emits Product\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("passthrough.forage"),
            "recipe \"passthrough\"\nengine http\n\
             share type Product { id: String }\n\
             input prior: [Product]\n\
             emits Product\n\
             for $p in $input.prior { emit Product { id ← $p.id } }\n",
        );
        write(
            &root.join("tail.forage"),
            "recipe \"tail\"\nengine http\n\
             share type Product { id: String }\n\
             input prior: [Product]\n\
             emits Product\n\
             for $p in $input.prior { emit Product { id ← $p.id } }\n",
        );
        write(
            &root.join("middle.forage"),
            "recipe \"middle\"\nengine http\n\
             compose \"leaf\" | \"passthrough\"\n",
        );
        write(
            &root.join("outer.forage"),
            "recipe \"outer\"\nengine http\n\
             compose \"middle\" | \"tail\"\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "outer").expect("link succeeds");
        assert!(
            !outcome.report.has_errors(),
            "errors: {:?}",
            outcome
                .report
                .issues
                .iter()
                .map(|i| (i.code, &i.message))
                .collect::<Vec<_>>(),
        );
        let module = outcome.module.expect("module produced");
        let mut names: Vec<&str> = module.stages.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, vec!["leaf", "middle", "passthrough", "tail"]);
    }

    /// A composition with an unknown stage name fails to validate.
    /// The report carries an `UnknownComposeStage` diagnostic; the
    /// module is `None`.
    #[test]
    fn link_unknown_stage_reports_diagnostic() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("composed.forage"),
            "recipe \"composed\"\nengine http\n\
             compose \"missing\" | \"alsoMissing\"\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "composed").expect("link runs");
        assert!(outcome.module.is_none());
        assert!(outcome.report.has_errors());
        assert!(
            outcome
                .report
                .issues
                .iter()
                .any(|i| matches!(i.code, crate::validate::ValidationCode::UnknownComposeStage)),
            "errors: {:?}",
            outcome.report.issues,
        );
    }

    /// A composition that references itself surfaces `ComposeCycle`
    /// on the root's report and produces no module. The grammar
    /// requires at least two stages, so the smallest cycle is `compose
    /// "a" | "b"` where `a` itself contains `compose "a" | "leaf"`.
    #[test]
    fn link_cyclic_composition_reports_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("leaf.forage"),
            "recipe \"leaf\"\nengine http\n\
             share type Product { id: String }\n\
             emits Product\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("a.forage"),
            "recipe \"a\"\nengine http\ncompose \"b\" | \"leaf\"\n",
        );
        write(
            &root.join("b.forage"),
            "recipe \"b\"\nengine http\ncompose \"a\" | \"leaf\"\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "a").expect("link runs");
        assert!(outcome.module.is_none());
        assert!(
            outcome
                .report
                .issues
                .iter()
                .any(|i| matches!(i.code, crate::validate::ValidationCode::ComposeCycle)),
            "errors: {:?}",
            outcome.report.issues,
        );
    }

    /// A composition that references a hub-dep stage surfaces
    /// `HubDepStageUnsupported` on the root's report.
    #[test]
    fn link_hub_dep_stage_reports_diagnostic() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("forage.toml"), STARTER_MANIFEST);
        write(
            &root.join("local.forage"),
            "recipe \"local\"\nengine http\n\
             share type Product { id: String }\n\
             emits Product\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("composed.forage"),
            "recipe \"composed\"\nengine http\n\
             compose \"@upstream/published\" | \"local\"\n",
        );
        let ws = load(root).unwrap();
        let outcome = link(&ws, "composed").expect("link runs");
        assert!(outcome.module.is_none());
        assert!(
            outcome.report.issues.iter().any(|i| matches!(
                i.code,
                crate::validate::ValidationCode::HubDepStageUnsupported
            )),
            "errors: {:?}",
            outcome.report.issues,
        );
    }

    /// `link_standalone` mirrors `link` for files outside a workspace.
    /// Composition stages can't resolve against anything, so a
    /// composed source surfaces `UnknownComposeStage`.
    #[test]
    fn link_standalone_rejects_composition_outside_workspace() {
        let src = "recipe \"composed\"\nengine http\ncompose \"first\" | \"second\"\n";
        let parsed = crate::parse(src).expect("parses");
        let outcome = link_standalone(parsed);
        assert!(outcome.module.is_none());
        assert!(
            outcome
                .report
                .issues
                .iter()
                .any(|i| matches!(i.code, crate::validate::ValidationCode::UnknownComposeStage)),
            "errors: {:?}",
            outcome.report.issues,
        );
    }

    /// A header-less file (no recipe) round-trips through
    /// `link_standalone` as a module — used by the LSP when validating
    /// a declarations-only sibling.
    #[test]
    fn link_standalone_succeeds_on_headerless_file() {
        let src = "share type Product { id: String }\n";
        let parsed = crate::parse(src).expect("parses");
        let outcome = link_standalone(parsed);
        assert!(!outcome.report.has_errors(), "errors: {:?}", outcome.report);
        let module = outcome.module.expect("module produced");
        assert!(module.stages.is_empty());
    }
}
