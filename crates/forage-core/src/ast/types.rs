//! Recipe-declared types, enums, and input/secret declarations.

use serde::{Deserialize, Serialize};

use crate::ast::alignment::AlignmentUri;
use crate::ast::span::Span;

/// A recipe-declared type. Recipes ship their own type catalog;
/// forage-core doesn't pre-define `Product` / `Variant` / etc.
///
/// `shared = true` (the `share type …` prefix) makes this declaration
/// visible to every other file in the workspace. Without it, the type
/// is file-scoped.
///
/// `alignments` are ontology correspondences declared between the type
/// keyword and the opening `{` — `aligns schema.org/Product`,
/// `aligns wikidata/Q2424752`, repeatable. Independent of `shared`:
/// a file-local type can carry alignments. The hub uses them for
/// discovery and JSON-LD output; the runtime carries them through to
/// the snapshot but does not transform values.
///
/// `extends` is the optional single-parent extension reference declared
/// between the type name and the alignments. The child inherits every
/// field of the parent plus the parent's type-level alignments; the
/// child can add fields, add type-level alignments, override a parent
/// field's alignment, or drop a parent field's alignment by
/// redeclaring the field without one. Field type narrowing is rejected
/// by the validator (`IncompatibleExtension`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeType {
    pub name: String,
    pub fields: Vec<RecipeField>,
    pub shared: bool,
    pub alignments: Vec<AlignmentUri>,
    pub extends: Option<TypeExtension>,
    /// Source range covering the whole `type Name { … }` block. Default
    /// (`0..0`) when constructed by hand.
    #[serde(default)]
    pub span: Span,
}

/// `extends [@author/]Name@vN` — one-shot reference to a parent type.
/// `author = None` is a workspace-local reference resolved against the
/// `TypeCatalog`; `author = Some(...)` is a hub-dep reference, also
/// resolved against the catalog (the workspace loader pre-folds
/// lockfile-pinned hub types into the catalog by bare name).
///
/// `version` is the parent type's hub version pin. For workspace-local
/// extension chains, the catalog has no version axis — the integer is
/// recorded for hub publishes and for cross-author bookkeeping but
/// doesn't drive resolution. For hub-dep extension, the validator
/// confirms the lockfile pin matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeExtension {
    pub author: Option<String>,
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub span: Span,
}

impl RecipeType {
    pub fn field(&self, name: &str) -> Option<&RecipeField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// One field inside a `RecipeType`. `alignment` is the optional
/// per-field ontology mapping declared with `aligns <uri>` after the
/// field type / optional marker (e.g. `name: String aligns schema.org/name`).
/// Limited to one per field — multi-ontology field correspondence is
/// out of scope until the hub side has reason for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeField {
    pub name: String,
    pub ty: FieldType,
    /// `name: Type?` — required vs optional.
    pub optional: bool,
    pub alignment: Option<AlignmentUri>,
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
