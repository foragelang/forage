# Types and enums

Recipes declare their own type catalog. The runtime has no built-in
`Product` / `Story` / `Item` — each recipe ships the shape it'll emit.

```forage
type Product {
    externalId:  String
    name:        String
    description: String?           // optional
    brand:       String?
    images:      [String]          // array
    category:    Category          // record reference
    menu:        MenuType          // enum reference
}

type Category {
    externalId: String
    name:       String
}

enum MenuType { RECREATIONAL, MEDICAL }
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

Required fields must be bound in every emit of that type; the validator
flags missing ones with `MissingRequiredField`. Optional fields default
to `null` when unbound.

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
