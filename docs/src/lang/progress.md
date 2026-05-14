# Progress

A recipe's **progress unit** is the iteration scope whose completion
represents one "unit of work." UIs (Forage Studio's progress bar, CLI
run reporters) scope progress to that unit instead of summing every
emitted record across types — so "1,346 Products" matches the author's
mental model, not "4,059 records" of mixed Product / Variant /
PriceObservation.

The unit is **inferred from the recipe's shape**. No keyword, no
annotation — restructure the loops and the unit moves with the code.

## The rule

> **The unit is the outermost `for` loop that both (a) emits records
> directly and (b) contains nested `for` loops that also emit.**
>
> Such a loop is a *compound unit*: each iteration produces a
> top-level record plus all of its nested children. The iteration
> isn't done until its nested emits are done, so counting outer
> iterations is honest progress.
>
> **Fallback** when no compound exists: the outermost emit-bearing
> `for` loop. **Final fallback**: top-level emits → one run = one
> unit.

Ties at the same depth go to **last-in-source-order** — sequential
sibling loops execute one after another, so the final one is the
bottleneck for completion.

## Example: zen-leaf

```forage
recipe "zen-leaf-elkridge"
engine http

emit Dispensary { ... }                           // top-level

for $menu in $input.menuTypes {                   // no direct emit
    step categories { ... }

    for $cat in $categories[*] {                  // leaf: only Category
        emit Category { ... }
    }

    for $catId in $input.priceCategoryIds {       // no direct emit
        step products { ... paginate ... }

        for $product in $products[*] {            // ← UNIT (compound)
            emit Product { ... }

            for $variant in $product.variants[*] {   // nested leaf
                emit Variant { ... }
                emit PriceObservation { ... }
            }
        }
    }
}
```

Walk:

- `$menu` — no direct emit. Skip.
- `$cat` — direct emit (`Category`) but no nested emit-bearing
  loop. *Leaf*, not compound.
- `$catId` — no direct emit. Skip.
- `$product` — direct emit (`Product`) **and** nested
  `for $variant` which emits Variant + PriceObservation.
  ✓ **Compound. Unit = `$product`.**
- `$variant` — direct emits but no nested. Leaf.

Bar reads "**452 / 1,346 Product · 34%**" — one entry per iteration
of `$product`, regardless of how many Variants / PriceObservations
each one fans out into.

## Why not the innermost loop?

The innermost emit-bearing loop in zen-leaf is `$variant`. Picking
that gives "452 / 1,346 Variant" — which says nothing about how many
*products* are done. Worse, Variants and PriceObservations interleave
in the activity log so the displayed type flashes. Counting outer
iterations is more honest about overall completion.

## Why prefer compound over a sibling leaf?

In zen-leaf, `$cat` (depth 2, leaf-only) and `$product` (depth 3,
compound) are both emit-bearing. The compound rule prefers
`$product` because each iteration of `$product` is a richer unit of
work — it emits a Product *and* drives N Variants and N
PriceObservations underneath. The Category branch is bookkeeping.

## Refactor to shift the unit

The unit follows the structure. Hoist emits out a level and the unit
moves up; nest deeper and it moves down.

**Original — unit = Product:**

```forage
for $cat in $categories[*] {
    for $prod in $cat.products[*] {           // unit = $prod
        emit Product { ... }
        for $variant in $prod.variants[*] {
            emit Variant { ... }
        }
    }
}
```

**Hoisted — unit = Category** (bundle products as a field on
Category, drop the inner loop):

```forage
for $cat in $categories[*] {                  // ← unit = $cat
    emit Category {
        ...$cat,
        products: $cat.products,
    }
}
```

`$cat` is now a leaf (no nested emit-bearing loop) but it's also the
*only* emit-bearing loop, so the fallback rule picks it. Bar: "7 /
20 Category · 35%."

**Hoisted further — unit = recipe** (single top-level emit):

```forage
emit Workspace {
    categories: $categories,
    products:   $allProducts,
}
```

No emit-bearing loop at all. Final fallback: recipe scope, one run =
one unit.

## Sequential siblings

When two sibling `for` loops at the same depth both contain emits,
the **last** one in source order is the unit. Sequential loops
execute one after another — the final loop is the bottleneck for
"the recipe is finished."

```forage
for $a in $A[*] {           // runs first
    emit A { ... }
}
for $b in $B[*] {           // ← unit: the bottleneck
    emit B { ... }
}
```

## Top-level emits coexist with looped emits

A top-level `emit` (e.g. a single `Dispensary` for the whole run)
doesn't override the loop-based unit. It contributes one record to
the total but the unit is still the loop:

```forage
emit Dispensary { ... }                       // 1 record, not the unit

for $product in $products[*] {                // unit = $product
    emit Product { ... }
    for $variant in $product.variants[*] {
        emit Variant { ... }
    }
}
```

The progress bar tracks `$product` iterations; the `Dispensary` emit
contributes to the per-type breakdown but doesn't shape progress.
