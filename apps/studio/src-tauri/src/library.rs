//! Filesystem-backed recipe library at `~/Library/Forage/Recipes/<slug>/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// On-disk location of the user's recipe library.
///
/// Honors the `FORAGE_LIBRARY_ROOT` env var first — useful for tests
/// (sandbox into a tempdir) and for users who want to point Studio at a
/// repo checkout instead of the OS-conventional library directory.
pub fn library_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("FORAGE_LIBRARY_ROOT") {
        if !override_dir.is_empty() {
            return PathBuf::from(override_dir);
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library").join("Forage").join("Recipes");
        }
    }
    if let Some(data) = dirs::data_dir() {
        return data.join("Forage").join("Recipes");
    }
    PathBuf::from(".forage-recipes")
}

#[derive(Debug, Serialize, Clone)]
pub struct RecipeEntry {
    pub slug: String,
    pub path: PathBuf,
    pub has_fixtures: bool,
}

pub fn list_entries() -> Vec<RecipeEntry> {
    let root = library_root();
    let _ = fs::create_dir_all(&root);
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir(&root) else {
        return out;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let recipe_path = path.join("recipe.forage");
        if !recipe_path.exists() {
            continue;
        }
        let slug = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
            .unwrap_or_default();
        let has_fixtures = path.join("fixtures").join("captures.jsonl").exists()
            || path.join("fixtures").join("inputs.json").exists();
        out.push(RecipeEntry {
            slug,
            path,
            has_fixtures,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

pub fn recipe_path(slug: &str) -> PathBuf {
    library_root().join(slug).join("recipe.forage")
}

pub fn recipe_dir(slug: &str) -> PathBuf {
    library_root().join(slug)
}

pub fn create_recipe(template_slug: Option<&str>) -> std::io::Result<String> {
    let root = library_root();
    fs::create_dir_all(&root)?;
    // Find an `untitled-N` slug that doesn't exist yet.
    let base = template_slug.unwrap_or("untitled");
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{base}-1")
        } else {
            format!("{base}-{n}")
        };
        let candidate_path = root.join(&candidate);
        if !candidate_path.exists() {
            fs::create_dir_all(candidate_path.join("fixtures"))?;
            let source = format!(
                "recipe \"{candidate}\" {{\n    engine http\n\n    type Item {{\n        id: String\n    }}\n\n    step list {{\n        method \"GET\"\n        url    \"https://example.com\"\n    }}\n\n    for $i in $list.items[*] {{\n        emit Item {{\n            id ← $i.id\n        }}\n    }}\n}}\n"
            );
            fs::write(candidate_path.join("recipe.forage"), source)?;
            return Ok(candidate);
        }
        n += 1;
        if n > 1000 {
            return Err(std::io::Error::other("too many untitled recipes"));
        }
    }
}

pub fn read_source(slug: &str) -> std::io::Result<String> {
    fs::read_to_string(recipe_path(slug))
}

/// Delete a recipe directory under the library root.
///
/// Refuses anything that isn't a single path segment (no slashes, no `..`),
/// so a malicious slug can't escape the library root with `../etc/passwd`.
/// The slug must already exist as a directory directly under the library.
pub fn delete_recipe(slug: &str) -> std::io::Result<()> {
    delete_recipe_in(&library_root(), slug)
}

/// Test-friendly variant of `delete_recipe` that takes an explicit root.
fn delete_recipe_in(root: &Path, slug: &str) -> std::io::Result<()> {
    if slug.is_empty() || slug.contains('/') || slug.contains('\\') || slug == "." || slug == ".." {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid recipe slug: {slug:?}"),
        ));
    }
    let dir = root.join(slug);
    // Confirm the target sits inside the library root before deleting — a
    // hardlink or symlink would otherwise let us nuke unrelated content.
    let canonical = dir.canonicalize()?;
    let root_canonical = root.canonicalize()?;
    if !canonical.starts_with(&root_canonical) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("recipe path {canonical:?} escapes library root {root_canonical:?}"),
        ));
    }
    fs::remove_dir_all(&dir)
}

pub fn write_source(slug: &str, source: &str) -> std::io::Result<()> {
    let path = recipe_path(slug);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, source)
}

pub fn read_inputs(slug: &str) -> indexmap::IndexMap<String, serde_json::Value> {
    let path = recipe_dir(slug).join("fixtures").join("inputs.json");
    if !path.exists() {
        return indexmap::IndexMap::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return indexmap::IndexMap::new(),
    };
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let mut out = indexmap::IndexMap::new();
    if let serde_json::Value::Object(o) = v {
        for (k, v) in o {
            out.insert(k, v);
        }
    }
    out
}

pub fn read_captures(slug: &str) -> Vec<forage_replay::Capture> {
    let path = recipe_dir(slug).join("fixtures").join("captures.jsonl");
    if !path.exists() {
        return Vec::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(c) = serde_json::from_str::<forage_replay::Capture>(line) {
            out.push(c);
        }
    }
    out
}

/// Convenience for `forage_keychain` env-style secret resolution.
pub fn read_secrets_from_env(recipe: &forage_core::Recipe) -> indexmap::IndexMap<String, String> {
    let mut out = indexmap::IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}

#[allow(dead_code)]
pub fn ensure_path<P: AsRef<Path>>(p: P) -> std::io::Result<()> {
    if let Some(parent) = p.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_recipe(root: &Path, slug: &str) {
        let dir = root.join(slug);
        fs::create_dir_all(dir.join("fixtures")).unwrap();
        fs::write(dir.join("recipe.forage"), "recipe \"x\" { engine http }").unwrap();
        fs::write(dir.join("fixtures").join("inputs.json"), "{}").unwrap();
    }

    #[test]
    fn delete_removes_directory_and_fixtures() {
        let tmp = tempfile::tempdir().unwrap();
        make_recipe(tmp.path(), "to-delete");
        assert!(tmp.path().join("to-delete/recipe.forage").exists());
        assert!(tmp.path().join("to-delete/fixtures/inputs.json").exists());

        delete_recipe_in(tmp.path(), "to-delete").unwrap();

        assert!(!tmp.path().join("to-delete").exists());
    }

    #[test]
    fn delete_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let siblings = tempfile::tempdir().unwrap();
        let victim = siblings.path().join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("important.txt"), "DO NOT DELETE").unwrap();

        for bad in ["..", "../victim", "./x", "a/b", "a\\b", ""] {
            let err = delete_recipe_in(tmp.path(), bad).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "slug {bad:?}");
        }
        assert!(victim.join("important.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn delete_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("important.txt"), "DO NOT DELETE").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("evil")).unwrap();

        let err = delete_recipe_in(tmp.path(), "evil").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(outside.path().join("important.txt").exists());
    }
}
