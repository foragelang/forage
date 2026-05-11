// Public surface — parse, validate, run, publish.

export * from './ast.js'
export { Lexer, LexError, KEYWORDS } from './lexer.js'
export type { Token, TokenKind, SourceLoc } from './lexer.js'
export { Parser, ParseError } from './parser.js'
export { validate, hasErrors } from './validator.js'
export type { ValidationIssue } from './validator.js'
export { TransformImpls, TransformError, fromRawJSON, toRawJSON } from './transforms.js'
export { Scope, ExtractionEvaluator, resolvePath, stringifyJSON, ScopeError, EvaluationError } from './extraction.js'
export { run } from './runner.js'
export type { RunResult, RunOptions, RunDiagnostic, ScrapedRecord, FetchLike } from './runner.js'
export { HubClient, DEFAULT_HUB_API } from './hub-client.js'
export type {
    RecipeListItem,
    RecipeDetail,
    PublishPayload,
    PublishResult,
    HubClientOptions,
} from './hub-client.js'

import { Parser } from './parser.js'
import type { Recipe } from './ast.js'

/** Convenience: parse a source string. Throws ParseError on failure. */
export function parse(source: string): Recipe {
    return Parser.parse(source)
}
