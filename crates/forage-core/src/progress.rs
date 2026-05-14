//! Infer the **progress unit** of a recipe from its iteration
//! structure. The unit is the smallest iteration scope that emits
//! records — each iteration of that scope is one "unit of work."
//!
//! Recipes typically nest loops to walk a hierarchy
//! (e.g. menus → categories → products) and emit records at the
//! deepest level. UIs that show "you're 36% done" need to know
//! whether 36% means "of total records" (raw emit count) or "of
//! products" (the thing the user thinks of as a unit). The shape of
//! the recipe already encodes that distinction: the deepest emit-
//! bearing `for` loop is the unit. No annotation needed —
//! restructure the loops and the unit moves with the code.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{Recipe, Statement};

/// The unit of work for one iteration of a recipe.
///
/// `variable` is the loop variable's name (without leading `$`).
/// `None` means the unit is the whole recipe — only happens when no
/// `for` loop contains an `emit`.
///
/// `types` lists the record types emitted inside the unit scope, in
/// source order. UIs typically use the first as the progress
/// denominator (they all share the same iteration count, modulo
/// conditional emits inside the scope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProgressUnit {
    pub variable: Option<String>,
    pub types: Vec<String>,
}

/// Walk `recipe.body` and return the unit info.
///
/// Rule (in priority order):
///
/// 1. **Outermost compound** — a `for` loop that emits records
///    directly *and* contains nested emit-bearing `for` loops. This
///    captures the "Product → Variant → PriceObservation" pattern:
///    one Product iteration is the natural unit of work, even though
///    it emits Variants/Prices underneath. The Product iteration
///    isn't done until its nested loops are done, so progress in
///    Products is honest progress.
///
/// 2. **Outermost emit-bearing `for` loop** — fallback when no
///    compound exists. The recipe has emits in loops but no nested
///    emit structure.
///
/// 3. **Recipe scope** — only top-level emits, no emit-bearing
///    loops. One run = one unit.
///
/// Ties at the same depth go to the last-in-source-order (sequential
/// loops execute one after another; the final one is the bottleneck
/// for completion).
pub fn infer_progress_unit(recipe: &Recipe) -> Option<ProgressUnit> {
    let mut compounds: Vec<(usize, ProgressUnit)> = Vec::new();
    let mut emit_bearings: Vec<(usize, ProgressUnit)> = Vec::new();
    collect_candidates(&recipe.body, 0, &mut compounds, &mut emit_bearings);

    if let Some(u) = pick_outermost_last_wins(compounds) {
        return Some(u);
    }
    if let Some(u) = pick_outermost_last_wins(emit_bearings) {
        return Some(u);
    }
    let here: Vec<String> = recipe
        .body
        .iter()
        .filter_map(|s| match s {
            Statement::Emit(e) => Some(e.type_name.clone()),
            _ => None,
        })
        .collect();
    if !here.is_empty() {
        return Some(ProgressUnit {
            variable: None,
            types: here,
        });
    }
    None
}

/// Walk every for-loop and classify it: a candidate is "compound"
/// when it has direct emits and at least one descendant for-loop
/// that also emits, otherwise it's a plain "emit-bearing" candidate
/// (if it emits at all). Each candidate carries its depth so the
/// caller can pick outermost.
fn collect_candidates(
    body: &[Statement],
    depth: usize,
    compounds: &mut Vec<(usize, ProgressUnit)>,
    emit_bearings: &mut Vec<(usize, ProgressUnit)>,
) {
    for stmt in body {
        if let Statement::ForLoop {
            variable,
            body: inner,
            ..
        } = stmt
        {
            let direct: Vec<String> = inner
                .iter()
                .filter_map(|s| match s {
                    Statement::Emit(e) => Some(e.type_name.clone()),
                    _ => None,
                })
                .collect();
            let has_nested_emit_loop = inner
                .iter()
                .any(|s| matches!(s, Statement::ForLoop { body, .. } if contains_emit(body)));

            if !direct.is_empty() {
                let info = ProgressUnit {
                    variable: Some(variable.clone()),
                    types: direct,
                };
                if has_nested_emit_loop {
                    compounds.push((depth + 1, info));
                } else {
                    emit_bearings.push((depth + 1, info));
                }
            }
            collect_candidates(inner, depth + 1, compounds, emit_bearings);
        }
    }
}

fn contains_emit(body: &[Statement]) -> bool {
    body.iter().any(|s| match s {
        Statement::Emit(_) => true,
        Statement::ForLoop { body, .. } => contains_emit(body),
        _ => false,
    })
}

/// Pick the candidate at the smallest depth; tie-break by
/// last-in-source-order.
fn pick_outermost_last_wins(
    candidates: Vec<(usize, ProgressUnit)>,
) -> Option<ProgressUnit> {
    let min_depth = candidates.iter().map(|(d, _)| *d).min()?;
    candidates
        .into_iter()
        .filter(|(d, _)| *d == min_depth)
        .last()
        .map(|(_, u)| u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn unit_of(src: &str) -> Option<ProgressUnit> {
        let recipe = parse(src).expect("parse failed");
        infer_progress_unit(&recipe)
    }

    #[test]
    fn unit_is_innermost_emit_bearing_loop() {
        let src = r#"
            recipe "r"
            engine http

            type Cat { id: String }
            type Prod { id: String }

            step s {
                method "GET"
                url "https://x.test"
            }

            for $cat in $s[*] {
                for $prod in $cat.products[*] {
                    emit Prod { id ← $prod.id }
                }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("prod"));
        assert_eq!(unit.types, vec!["Prod"]);
    }

    #[test]
    fn outermost_compound_wins_over_inner_leaf() {
        // `$cat` is compound (direct Cat emit + nested $prod that
        // emits). `$prod` is a leaf emitter. The compound wins, so
        // unit = $cat, not $prod.
        let src = r#"
            recipe "r"
            engine http

            type Cat { id: String }
            type Prod { id: String }

            step s {
                method "GET"
                url "https://x.test"
            }

            emit Cat { id ← "root" }
            for $cat in $s[*] {
                emit Cat { id ← $cat.id }
                for $prod in $cat.products[*] {
                    emit Prod { id ← $prod.id }
                }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("cat"));
        assert_eq!(unit.types, vec!["Cat"]);
    }

    #[test]
    fn zen_leaf_pattern_picks_product_not_category() {
        // Mirrors zen-leaf-elkridge: a parallel branch emits Category
        // as a leaf and another branch nests products → variants.
        // The compound rule picks `$product` over `$cat` because
        // `$product` has nested emit-bearing structure underneath.
        let src = r#"
            recipe "r"
            engine http

            type Cat { id: String }
            type Prod { id: String }
            type Variant { id: String }

            step categories {
                method "GET"
                url "https://x.test/c"
            }
            step products {
                method "GET"
                url "https://x.test/p"
            }

            for $menu in $categories[*] {
                for $cat in $categories[*] {
                    emit Cat { id ← $cat.id }
                }
                for $catId in $categories[*] {
                    for $product in $products[*] {
                        emit Prod { id ← $product.id }
                        for $variant in $product.variants[*] {
                            emit Variant { id ← $variant.id }
                        }
                    }
                }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("product"));
        assert_eq!(unit.types, vec!["Prod"]);
    }

    #[test]
    fn top_level_emits_only_yields_recipe_scope() {
        let src = r#"
            recipe "r"
            engine http

            type Cat { id: String }

            emit Cat { id ← "a" }
            emit Cat { id ← "b" }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert!(unit.variable.is_none());
        assert_eq!(unit.types, vec!["Cat", "Cat"]);
    }

    #[test]
    fn no_emits_returns_none() {
        let src = r#"
            recipe "r"
            engine http
        "#;
        assert!(unit_of(src).is_none());
    }

    #[test]
    fn sibling_loops_at_same_depth_last_wins() {
        // Two sibling for-loops both contain emits at the same depth.
        // Source order resolves the tie — the *last* sibling is the
        // unit, since sequential loops execute one after another and
        // the final one is the bottleneck for run completion.
        let src = r#"
            recipe "r"
            engine http

            type A { id: String }
            type B { id: String }

            step s {
                method "GET"
                url "https://x.test"
            }

            for $a in $s[*] {
                emit A { id ← $a.id }
            }
            for $b in $s[*] {
                emit B { id ← $b.id }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("b"));
        assert_eq!(unit.types, vec!["B"]);
    }

    #[test]
    fn empty_for_loops_dont_count() {
        // A deeper for-loop with no emits inside is ignored — the unit
        // is the deepest *emit-bearing* loop, not the deepest loop.
        let src = r#"
            recipe "r"
            engine http

            type Prod { id: String }

            step s {
                method "GET"
                url "https://x.test"
            }

            for $cat in $s[*] {
                emit Prod { id ← $cat.id }
                for $unused in $cat.things[*] {
                    // nothing emitted here
                }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("cat"));
        assert_eq!(unit.types, vec!["Prod"]);
    }

    #[test]
    fn multiple_emits_in_unit_scope_keep_source_order() {
        let src = r#"
            recipe "r"
            engine http

            type Prod { id: String }
            type Variant { id: String }
            type Price { id: String }

            step s {
                method "GET"
                url "https://x.test"
            }

            for $prod in $s[*] {
                emit Prod { id ← $prod.id }
                emit Variant { id ← $prod.id }
                emit Price { id ← $prod.id }
            }
        "#;
        let unit = unit_of(src).expect("should infer");
        assert_eq!(unit.variable.as_deref(), Some("prod"));
        assert_eq!(unit.types, vec!["Prod", "Variant", "Price"]);
    }
}
