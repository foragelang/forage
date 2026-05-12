/* tslint:disable */
/* eslint-disable */

export function forage_version(): string;

/**
 * One-shot: parse + validate. Useful for the editor's hot path so the
 * JS side doesn't have to JSON-bridge the AST.
 */
export function parse_and_validate(source: string): any;

/**
 * Parse a recipe and return JSON: either the AST or a structured error.
 *
 * Shape on success:
 *   { ok: true, recipe: <Recipe as JSON> }
 * Shape on failure:
 *   { ok: false, error: { message, span?: { start, end } } }
 */
export function parse_recipe(source: string): any;

/**
 * Validate a recipe given its AST as JSON. Returns
 *   { errors: [...], warnings: [...] }
 */
export function validate_recipe(recipe_json: string): any;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly forage_version: (a: number) => void;
    readonly parse_and_validate: (a: number, b: number) => number;
    readonly parse_recipe: (a: number, b: number) => number;
    readonly validate_recipe: (a: number, b: number) => number;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export2: (a: number, b: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number, d: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
