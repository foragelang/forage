// AST types — match the Swift Recipe / Statement / Extraction shapes.
// All types are JSON-serializable.

export type EngineKind = 'http' | 'browser'

export type FieldType =
    | { tag: 'string' }
    | { tag: 'int' }
    | { tag: 'double' }
    | { tag: 'bool' }
    | { tag: 'array'; element: FieldType }
    | { tag: 'record'; name: string }
    | { tag: 'enumRef'; name: string }

export interface RecipeField {
    name: string
    type: FieldType
    optional: boolean
}

export interface RecipeType {
    name: string
    fields: RecipeField[]
}

export interface RecipeEnum {
    name: string
    variants: string[]
}

export interface InputDecl {
    name: string
    type: FieldType
    optional: boolean
}

export type AuthStrategy =
    | { tag: 'staticHeader'; name: string; value: Template }
    | { tag: 'htmlPrime'; stepName: string; capturedVars: HtmlPrimeVar[] }

export interface HtmlPrimeVar {
    varName: string
    regexPattern: string
    groupIndex: number
}

export type PathExpr =
    | { tag: 'current' }
    | { tag: 'input' }
    | { tag: 'variable'; name: string }
    | { tag: 'field'; base: PathExpr; name: string }
    | { tag: 'optField'; base: PathExpr; name: string }
    | { tag: 'index'; base: PathExpr; idx: number }
    | { tag: 'wildcard'; base: PathExpr }

export type JSONValue =
    | { tag: 'null' }
    | { tag: 'bool'; value: boolean }
    | { tag: 'int'; value: number }
    | { tag: 'double'; value: number }
    | { tag: 'string'; value: string }
    | { tag: 'array'; items: JSONValue[] }
    | { tag: 'object'; entries: Record<string, JSONValue> }

export type TemplatePart =
    | { tag: 'literal'; value: string }
    | { tag: 'interp'; expr: ExtractionExpr }

export interface Template {
    parts: TemplatePart[]
}

export type ExtractionExpr =
    | { tag: 'path'; path: PathExpr }
    | { tag: 'pipe'; inner: ExtractionExpr; calls: TransformCall[] }
    | { tag: 'caseOf'; scrutinee: PathExpr; branches: Array<{ label: string; expr: ExtractionExpr }> }
    | { tag: 'mapTo'; path: PathExpr; emission: Emission }
    | { tag: 'literal'; value: JSONValue }
    | { tag: 'template'; template: Template }
    | { tag: 'call'; name: string; args: ExtractionExpr[] }

export interface TransformCall {
    name: string
    args: ExtractionExpr[]
}

export interface FieldBinding {
    fieldName: string
    expr: ExtractionExpr
}

export interface Emission {
    typeName: string
    bindings: FieldBinding[]
}

export type BodyValue =
    | { tag: 'templateString'; template: Template }
    | { tag: 'literal'; value: JSONValue }
    | { tag: 'path'; path: PathExpr }
    | { tag: 'object'; entries: HTTPBodyKV[] }
    | { tag: 'array'; items: BodyValue[] }
    | { tag: 'caseOf'; scrutinee: PathExpr; branches: Array<{ label: string; value: BodyValue }> }

export interface HTTPBodyKV {
    key: string
    value: BodyValue
}

export type HTTPBody =
    | { tag: 'jsonObject'; entries: HTTPBodyKV[] }
    | { tag: 'form'; entries: Array<{ key: string; value: BodyValue }> }
    | { tag: 'raw'; template: Template }

export interface HTTPRequest {
    method: string
    url: Template
    headers: Array<{ key: string; value: Template }>
    body: HTTPBody | null
}

export type Pagination =
    | {
        tag: 'pageWithTotal'
        itemsPath: PathExpr
        totalPath: PathExpr
        pageParam: string
        pageSize: number
        pageZeroIndexed: boolean
    }
    | {
        tag: 'untilEmpty'
        itemsPath: PathExpr
        pageParam: string
        pageZeroIndexed: boolean
    }
    | {
        tag: 'cursor'
        itemsPath: PathExpr
        cursorPath: PathExpr
        cursorParam: string
    }

export interface HTTPStep {
    name: string
    request: HTTPRequest
    pagination: Pagination | null
}

export type Statement =
    | { tag: 'step'; step: HTTPStep }
    | { tag: 'emit'; emission: Emission }
    | { tag: 'forLoop'; variable: string; collection: PathExpr; body: Statement[] }

export type ComparisonOp = '>=' | '>' | '<=' | '<' | '==' | '!='

export interface Expectation {
    kind: { tag: 'recordCount'; typeName: string; op: ComparisonOp; value: number }
}

export interface HubRecipeRef {
    slug: string
    version: number | null
}

// Browser config is parsed but not run in the TS port; we keep it as an
// opaque field so detail/edit pages can still display/round-trip browser
// recipes without the in-browser runner trying to execute them.
export interface BrowserConfig {
    initialURL: Template
    // The full structure is preserved as JSON for round-tripping; we don't
    // need it for HTTP runner. The IDE shows "Open in Toolkit" for these.
}

export interface Recipe {
    name: string
    engineKind: EngineKind
    types: RecipeType[]
    enums: RecipeEnum[]
    inputs: InputDecl[]
    auth: AuthStrategy | null
    body: Statement[]
    browser: BrowserConfig | null
    expectations: Expectation[]
    imports: HubRecipeRef[]
}
