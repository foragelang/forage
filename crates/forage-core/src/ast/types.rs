//! Recipe-declared types, enums, and input/secret declarations.

use serde::{Deserialize, Serialize};

use crate::ast::span::Span;

/// A recipe-declared type. Recipes ship their own type catalog;
/// forage-core doesn't pre-define `Product` / `Variant` / etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeType {
    pub name: String,
    pub fields: Vec<RecipeField>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeEnum {
    pub name: String,
    pub variants: Vec<String>,
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
