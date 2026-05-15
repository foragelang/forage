//! Editor intelligence — host-friendly entry points for hover,
//! completion, and document symbols.
//!
//! Each function takes a source string + (optionally) a line/column and
//! returns JSON-friendly results. No LSP protocol types; consumers
//! shape them into whatever wire format they speak. `server.rs` adapts
//! these into `tower-lsp` types for actual LSP clients; Studio calls
//! them directly through Tauri commands.

use serde::Serialize;
use ts_rs::TS;

use forage_core::ast::FieldType;
use forage_core::source::LineMap;
use forage_core::validate::BUILTIN_TRANSFORMS;
use forage_core::parse;

/// Markdown hover payload for the word at (line, col), 0-based.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct HoverInfo {
    pub markdown: String,
}

/// Compute hover info at (line, col) in `source`, 0-based. Returns
/// `None` if the position isn't on a recognizable word or the word
/// doesn't match any known symbol.
pub fn hover_at(source: &str, line: u32, col: u32) -> Option<HoverInfo> {
    let recipe = parse(source).ok();
    let word = word_at(source, line, col)?;

    // Built-in transforms.
    if BUILTIN_TRANSFORMS.contains(&word.as_str()) {
        return Some(HoverInfo {
            markdown: format!("**`{word}`** — built-in transform"),
        });
    }

    let r = recipe.as_ref()?;

    if let Some(ty) = r.types.iter().find(|t| t.name == word) {
        let fields: Vec<String> = ty
            .fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {}{}",
                    f.name,
                    field_type_label(&f.ty),
                    if f.optional { "?" } else { "" }
                )
            })
            .collect();
        return Some(HoverInfo {
            markdown: format!("**type `{}`**\n\n{{ {} }}", ty.name, fields.join(", ")),
        });
    }
    if let Some(inp) = r.inputs.iter().find(|i| i.name == word) {
        return Some(HoverInfo {
            markdown: format!("**input `{}`** — {}", inp.name, field_type_label(&inp.ty)),
        });
    }
    if let Some(en) = r.enums.iter().find(|e| e.name == word) {
        return Some(HoverInfo {
            markdown: format!("**enum `{}`** {{ {} }}", en.name, en.variants.join(" | ")),
        });
    }
    if r.secrets.iter().any(|s| s == &word) {
        return Some(HoverInfo {
            markdown: format!(
                "**secret `{word}`** — resolved from `FORAGE_SECRET_{}`",
                word.to_uppercase()
            ),
        });
    }
    // Step name?
    use forage_core::ast::Statement;
    fn find_step(body: &[Statement], name: &str) -> bool {
        body.iter().any(|s| match s {
            Statement::Step(step) => step.name == name,
            Statement::ForLoop { body, .. } => find_step(body, name),
            Statement::Emit(_) => false,
        })
    }
    if find_step(&r.body, &word) {
        return Some(HoverInfo {
            markdown: format!("**step `{word}`**"),
        });
    }

    None
}

/// Extract the identifier under (line, col) — `[A-Za-z0-9_]+`. Returns
/// the word as a String, or None if the position isn't on one.
fn word_at(source: &str, line: u32, col: u32) -> Option<String> {
    let line_str = source.lines().nth(line as usize)?;
    let col = col as usize;
    let bytes = line_str.as_bytes();
    if col > bytes.len() {
        return None;
    }
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut s = col;
    while s > 0 && is_word(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = col;
    while e < bytes.len() && is_word(bytes[e]) {
        e += 1;
    }
    if s == e {
        return None;
    }
    std::str::from_utf8(&bytes[s..e]).ok().map(String::from)
}

fn field_type_label(ty: &FieldType) -> String {
    match ty {
        FieldType::String => "String".into(),
        FieldType::Int => "Int".into(),
        FieldType::Double => "Double".into(),
        FieldType::Bool => "Bool".into(),
        FieldType::Array(inner) => format!("[{}]", field_type_label(inner)),
        FieldType::Record(name) => name.clone(),
        FieldType::EnumRef(name) => name.clone(),
        FieldType::Ref(name) => format!("Ref<{name}>"),
    }
}

/// Quick sanity check: convert a span end back to (line, col) for
/// consumers that need it without depending on forage-core directly.
pub fn position_for(source: &str, byte_offset: usize) -> (u32, u32) {
    let lm = LineMap::new(source);
    let p = lm.position(byte_offset);
    (p.line, p.character)
}
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"recipe "test"
engine http
type Item { id: String, name: String? }
input limit: Int
secret token

step list {
    method "GET"
    url "https://example.com"
}

for $i in $list[*] {
    emit Item { id ← $i.id | trim }
}
"#;

    fn line_col_of(needle: &str) -> (u32, u32) {
        // Helper: find the first column of `needle` and return its
        // (line, mid-column). Useful for "hover over X" tests.
        for (i, line) in FIXTURE.lines().enumerate() {
            if let Some(col) = line.find(needle) {
                let mid = col + needle.len() / 2;
                return (i as u32, mid as u32);
            }
        }
        panic!("needle {needle:?} not found in fixture");
    }

    #[test]
    fn hover_on_transform_returns_transform_doc() {
        let (l, c) = line_col_of("trim");
        let h = hover_at(FIXTURE, l, c).expect("hover");
        assert!(h.markdown.contains("trim"));
        assert!(h.markdown.contains("transform"));
    }

    #[test]
    fn hover_on_type_returns_field_signature() {
        let (l, c) = line_col_of("Item ");
        let h = hover_at(FIXTURE, l, c).expect("hover");
        assert!(h.markdown.contains("type"));
        assert!(h.markdown.contains("Item"));
        assert!(h.markdown.contains("id"));
        assert!(h.markdown.contains("name"));
    }

    #[test]
    fn hover_on_input_lists_type() {
        let (l, c) = line_col_of("limit");
        let h = hover_at(FIXTURE, l, c).expect("hover");
        assert!(h.markdown.contains("input"));
        assert!(h.markdown.contains("Int"));
    }

    #[test]
    fn hover_off_word_returns_none() {
        // Column 0 of a comment-only line in a non-existent position.
        assert!(hover_at(FIXTURE, 99, 0).is_none());
    }

    #[test]
    fn hover_on_step_name_identifies_step() {
        // Land in the middle of `list` (the step name), not on the
        // `step` keyword. The fixture has `step list {` at top level —
        // column 6 is the `i` of `list`.
        let h = hover_at(FIXTURE, 6, 6).expect("hover on 'list'");
        assert!(h.markdown.contains("step"));
        assert!(h.markdown.contains("list"));
    }
}
