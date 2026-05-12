//! Filesystem-backed recipe library at `~/Library/Forage/Recipes/<slug>/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// On-disk location of the user's recipe library.
pub fn library_root() -> PathBuf {
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
