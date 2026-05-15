//! Recipe-declared types, enums, and input/secret declarations.

use serde::{Deserialize, Serialize};

use crate::ast::span::Span;

/// A recipe-declared type. Recipes ship their own type catalog;
/// forage-core doesn't pre-define `Product` / `Variant` / etc.
///
/// `shared = true` (the `share type …` prefix) makes this declaration
/// visible to every other file in the workspace. Without it, the type
/// is file-scoped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeType {
    pub name: String,
    pub fields: Vec<RecipeField>,
    pub shared: bool,
    /// Source range covering the whole `type Name { … }` block. Default
    /// (`0..0`) when constructed by hand.
    #[serde(default)]
    pub span: Span,
}

impl RecipeType {
    pub fn field(&self, name: &str) -> Option<&RecipeField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeField {
    pub name: String,
    pub ty: FieldType,
    /// `name: Type?` — required vs optional.
    pub optional: bool,
}

/// Recipe field types. References to other recipe-declared types and enums
/// are by name; resolved at validation time, not in the parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldType {
    String,
    Int,
    Double,
    Bool,
    Array(Box<FieldType>),
    /// Named record reference.
    Record(String),
    /// Named enum reference.
    EnumRef(String),
    /// `Ref<T>` — typed reference to a record of type `T`. The value at
    /// runtime is the `_id` of an emitted record of that type, carried
    /// as `EvalValue::Ref` so the engine can serialize it as a typed
    /// pointer rather than a bare string FK.
    Ref(String),
}

/// `enum MenuType { RECREATIONAL, MEDICAL }`.
///
/// `shared = true` (the `share enum …` prefix) makes this declaration
/// visible to every other file in the workspace. Without it, the enum
/// is file-scoped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeEnum {
    pub name: String,
    pub variants: Vec<String>,
    pub shared: bool,
    #[serde(default)]
    pub span: Span,
}

/// Consumer-supplied input declaration. The runtime validates the supplied
/// inputs against these decls before running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputDecl {
    pub name: String,
    pub ty: FieldType,
    pub optional: bool,
    #[serde(default)]
    pub span: Span,
}

/// Recipe output signature — the union of types this recipe may `emit`.
/// `output T` is a single-type recipe; `output T1 | T2 | …` declares a
/// multi-type sum. The validator checks every `emit X { … }` against the
/// declared set and rejects emissions of types not listed here.
///
/// `types` is the unresolved list as written by the author. The validator
/// resolves each name against the recipe's `TypeCatalog` before checking
/// emissions — unknown names surface as `UnknownType` on the output decl,
/// not as silent passes through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputDecl {
    pub types: Vec<String>,
    #[serde(default)]
    pub span: Span,
}
