/// recipeNameOf joins workspace-relative paths against the parsed
/// recipe statuses. Tests pin the contract callers rely on:
///   * exact path match resolves to the recipe header name;
///   * broken or missing-draft entries don't return a name even
///     when their path field would match;
///   * unknown paths and empty inputs return null.

import { describe, expect, test } from "vitest";

import type { RecipeStatus } from "../bindings/RecipeStatus";
import { recipeNameOf } from "./path";

const valid = (name: string, path: string): RecipeStatus => ({
    name,
    draft: { kind: "valid", path },
    deployed: { kind: "none" },
});

const broken = (name: string, path: string): RecipeStatus => ({
    name,
    draft: { kind: "broken", path, error: "parse error" },
    deployed: { kind: "none" },
});

const missing = (name: string): RecipeStatus => ({
    name,
    draft: { kind: "missing" },
    deployed: { kind: "deployed", version: 1, deployed_at: 0 },
});

describe("recipeNameOf", () => {
    test("returns the header name when the path matches a valid draft", () => {
        const recipes = [valid("bar", "foo.forage")];
        expect(recipeNameOf("foo.forage", recipes)).toBe("bar");
    });

    test("matches against the workspace-relative path shape that FileNode uses", () => {
        // Path matching is exact; the helper joins whatever path the
        // backend hands it (`RecipeStatus.draft.path`) against the
        // path argument. Two recipes with distinct file paths each
        // resolve to their own header name.
        const recipes = [
            valid("first", "first.forage"),
            valid("second", "second.forage"),
        ];
        expect(recipeNameOf("first.forage", recipes)).toBe("first");
        expect(recipeNameOf("second.forage", recipes)).toBe("second");
    });

    test("returns null for a path that isn't in the workspace", () => {
        const recipes = [valid("bar", "foo.forage")];
        expect(recipeNameOf("other.forage", recipes)).toBeNull();
    });

    test("returns null when the path belongs to a broken draft", () => {
        // A broken draft has no usable recipe header; callers can't
        // call recipe-scoped commands against it. The path field
        // exists for the file tree, not for the wire.
        const recipes = [broken("filename-stem", "broken.forage")];
        expect(recipeNameOf("broken.forage", recipes)).toBeNull();
    });

    test("missing drafts (deploy without source) never resolve a path", () => {
        const recipes = [missing("ghost")];
        expect(recipeNameOf("ghost.forage", recipes)).toBeNull();
    });

    test("undefined recipes (query loading) returns null", () => {
        expect(recipeNameOf("foo.forage", undefined)).toBeNull();
    });
});
