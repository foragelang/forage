# Types and enums

Recipes declare their own type catalog. The runtime has no built-in
`Product` / `Story` / `Item` — each recipe ships the shape it'll emit,
or pulls it in from a `share`d declaration somewhere in the workspace.

```forage
share type Product {
    externalId:  String
    name:        String
    description: String?           // optional
    brand:       String?
    images:      [String]          // array
    category:    Category          // record reference
    menu:        MenuType          // enum reference
}

share type Category {
    externalId: String
    name:       String
}

share enum MenuType { RECREATIONAL, MEDICAL }
```

By default a `type` or `enum` is **file-scoped** — visible only to
declarations in the same file (and to the recipe declared there, if
any). Prefixing the declaration with `share` publishes it to the
workspace-wide catalog visible to every other `.forage` file.

Workspace-wide name collisions among `share`d declarations are a
validator error. A file-scoped declaration overrides a same-named
`share`d declaration when both reach the same recipe's catalog — useful
for recipe-specific overrides of a shared shape.

```forage
share type Product {                  // workspace-visible default shape
    externalId: String
    sku:        String
}

// elsewhere in the workspace — alongside a different recipe:
type Product {                        // overrides the share above
    externalId: String
    sku:        String
    terpenes:   [String]              // extra fields just for this recipe
}
```

## Field types

- **Primitives**: `String`, `Int`, `Double`, `Bool`.
- **Optional**: `T?` — the field is nullable. Omitting it in an emit
  block is legal.
- **Array**: `[T]` — homogeneous list; nestable (`[[Double]]`).
- **Record reference**: bare TypeName — links to another `type` declared
  in the same recipe.
- **Enum reference**: bare EnumName — must match an `enum` in the same
  recipe; the value must be one of the listed variants.
- **Typed reference**: `Ref<TypeName>` — a typed pointer to an emitted
  record of `TypeName`. See below.

Required fields must be bound in every emit of that type; the validator
flags missing ones with `MissingRequiredField`. Optional fields default
to `null` when unbound.

## Typed references — `Ref<T>`

Recipes that emit related records (a `Product` and its `Variant`s, a
`Variant` and its `PriceObservation`s) use `Ref<T>` to make the link
explicit at the type level instead of carrying string foreign keys:

```forage
type Product { externalId: String, name: String }

type Variant {
    product:    Ref<Product>       // typed link, not a string FK
    externalId: String
    name:       String?
}
```

Every `Ref<T>` field must be *explicitly bound* at every emit site —
there's no implicit-null even for optional refs. The right-hand side
must be a binding introduced by a prior `emit T { … } as $name` in the
same lexical scope:

```forage
for $p in $products[*] {
    emit Product {
        externalId ← $p.id | toString
        name       ← $p.name
    } as $prod

    for $v in $p.variants[*] {
        emit Variant {
            product    ← $prod          // type-checked: $prod is Ref<Product>
            externalId ← $v.id | toString
        }
    }
}
```

The validator enforces:

- The target type (`Ref<X>`) exists.
- Every `Ref<T>` field is bound at every emit site
  (`MissingRefAssignment` otherwise).
- The bound expression resolves to an `emit T { … } as $name` binding of
  matching type (`RefTypeMismatch` otherwise — literals, templates, and
  arbitrary path expressions are rejected).
- `as $name` doesn't shadow another binding in scope
  (`DuplicateBinding` otherwise).

At runtime, the engine assigns each emitted record a synthetic
sequential `_id` (`rec-0`, `rec-1`, …) and stores `Ref` field values as
`{"_ref": "rec-N", "_type": "Product"}` JSON objects inside the
record's `fields` blob. The Studio output viewer renders refs as
`→ Product(rec-N)` so the parent-child link reads at a glance.

## Enums

Enums are closed sets of named variants:

```forage
enum MenuType { RECREATIONAL, MEDICAL }
enum StrainPrevalence { INDICA, SATIVA, HYBRID, CBD }
```

The validator treats variant names case-sensitively. `case` expressions
exhaustively dispatch on enum values:

```forage
price ← case $menu of {
    RECREATIONAL → $variant.priceRec
    MEDICAL      → $variant.priceMed
}
```

Missing variants in a `case-of` are warnings, not errors — sometimes you
genuinely want to handle a subset. Unknown labels (not in the enum) are
errors.

## Forward references

Type and enum names resolve at validation time, not parse time, so the
order of declarations in a recipe doesn't matter:

```forage
type Product { category: Category }   // OK even though Category is below
type Category { name: String }
```

## Recipe output signature

A recipe declares the set of types it emits with a top-level `output`
clause. Single-type recipes use `output T`; recipes that emit several
types declare a sum with `|`:

```forage
recipe "products-and-prices"
engine http
output Product | Variant | PriceObservation

type Product { /* … */ }
type Variant { /* … */ }
type PriceObservation { /* … */ }
```

Every `emit X { … }` in the recipe body must reference a type listed in
the `output` clause; the validator rejects mismatches with
`MissingFromOutput`. Types listed but never emitted surface as
`UnusedInOutput` warnings. The output declaration is what the hub
indexes — `producers_of(Product)` resolves against the declared
signature, not the runtime emit set.

The clause is optional in the AST today; pre-migration recipes still
parse and validate without one. Future sub-plans tighten the
constraint when composition (`recipeA | recipeB`) lands and the
output signature becomes load-bearing for type-checking pipes.
