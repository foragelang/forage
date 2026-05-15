# Forage grammar

The complete grammar of the `.forage` language as implemented in
`crates/forage-core/src/parse/`. This document describes the surface
syntax — the parser produces an AST defined in
`crates/forage-core/src/ast/`. Expression *semantics* (numeric
coercion, regex dialect, string built-ins, etc.) live in
`notes/expression-language.md`.

Notation:

- `:=` introduces a production. `|` alternates. `?` is optional, `*`
  zero-or-more, `+` one-or-more, `(...)` groups.
- Terminals appear quoted (`'foo'`) or as uppercase token names
  (`STRING`, `INT`, etc.). Non-terminals are lowercase.
- The grammar is recursive-descent (`parse_*` functions in
  `crates/forage-core/src/parse/parser.rs`); no operator-precedence
  tables outside what's shown here.

## Lexical alphabet

Tokens produced by the lexer at `crates/forage-core/src/parse/lexer.rs`:

**Punctuation and brackets**

```
'{'  '}'  '('  ')'  '['  ']'  ','  ';'  ':'  '.'  '?'  '?.'  '[*]'
'|'  '←'  '→'  '='  '>'  '<'  '!'  '+'  '-'  '*'  '/'  '%'
```

`[*]` is a single token (the wildcard), not `[`+`*`+`]`. `←` (U+2190)
and `→` (U+2192) are single characters.

**Path heads**

```
'$'                  (DollarRoot — used as `$.field`)
'$input'             (DollarInput)
'$secret'            (DollarSecret — always followed by `.<name>`)
'$<ident>'           (DollarVar — captures the identifier)
```

**Literals**

```
STRING               "..." (may contain {...} interpolations)
INT                  -?[0-9]+
FLOAT                -?[0-9]+\.[0-9]+
BOOL                 'true' | 'false'
NULL                 'null'
DATE                 YYYY-MM-DD
REGEX                /pattern/flags        (flags ⊆ {i,m,s,u})
```

**Identifiers**

```
Ident                lowercase-starting identifier
TypeName             uppercase-starting identifier
Keyword              reserved word (see KEYWORDS list in token.rs)
```

Field-position identifiers may be keywords (`name`, `value`,
`headers`, etc.) — the parser accepts a keyword wherever a field
name is expected.

Comments: `//` to end-of-line and `/* ... */` block. Stripped by the
lexer.

## Top-level file

A `.forage` file is a flat sequence of top-level forms. There is
only one file format — files differ in *content*, not in *kind*. A
file that includes a `recipe_header` declares a recipe; one that
doesn't is a pure declarations file. The grammar is the same.

```
forage_file          := top_level_form*

top_level_form       := recipe_header
                      | type_decl
                      | enum_decl
                      | input_decl
                      | output_decl
                      | secret_decl
                      | fn_decl
                      | auth_block
                      | browser_block
                      | expect_block
                      | statement

recipe_header        := 'recipe' STRING 'engine' engine_kind

engine_kind          := 'http' | 'browser'

output_decl          := 'output' ( TypeName ( '|' TypeName )* )?
```

`output_decl` declares the recipe's output signature: `output
Product` for a single-type recipe; `output Product | Variant |
PriceObservation` for a multi-type sum. The `|` token is the same
one used by pipe expressions; the parser disambiguates by context
(only `TypeName`s are legal in this position).

Top-level forms appear flat at the file root — no surrounding `{ }`
block. Validator-enforced constraints (not parser-enforced):

- **At most one `recipe_header` per file.** A second header is a
  validator error.
- **Recipe-context forms require a header.** `auth_block`,
  `browser_block`, `expect_block`, `output_decl`, and `statement`s
  are only meaningful inside a recipe; if any appear in a file with
  no `recipe_header`, the validator rejects (`OutputWithoutHeader`
  for output specifically; `RecipeContextWithoutHeader` for the
  rest).
- **`output_decl` is at most one per file.** A second `output` clause
  is a parse error.
- **`output_decl` must list at least one type.** `output` with no
  TypeName parses but the validator emits `EmptyOutput`. Every
  type listed must resolve through the type catalog; unknown
  names surface as `UnknownType`. Every `emit T { … }` whose `T`
  is not in the declared list is rejected with `MissingFromOutput`.
  Listed types with no corresponding `emit` warn as
  `UnusedInOutput`. The output clause is optional in the AST; the
  validator skips the emit-vs-output check entirely when the clause
  is absent.
- **Order is free.** The header may appear anywhere among the other
  forms; the parser collects each kind into its slot on the
  `ForageFile` AST regardless of position.

## Type, enum, input, secret

```
type_decl            := 'share'? 'type' TypeName '{' field_list '}'

field_list           := ( field ( ';' | ',' )? )*

field                := field_name ':' field_type '?'?

field_name           := Ident | Keyword

field_type           := 'String' | 'Int' | 'Double' | 'Bool'
                      | '[' field_type ']'
                      | 'Ref' '<' TypeName '>'
                      | TypeName

enum_decl            := 'share'? 'enum' TypeName '{' enum_variants '}'

enum_variants        := ( variant ( ',' | ';' )? )*

variant              := Ident | TypeName

input_decl           := 'input' field_name ':' field_type '?'?

secret_decl          := 'secret' Ident
```

`field_type`'s bare `TypeName` is either a record reference or an
enum reference — the validator resolves it against the type catalog.
The lexer's `TYPE_KEYWORDS` (`String`/`Int`/`Double`/`Bool`) are
reserved as keywords; user types are arbitrary uppercase identifiers.

`share` is an optional visibility marker that prefixes `type`,
`enum`, or `fn`. Without `share`, the declaration is *file-scoped* —
visible only inside the same file (i.e. to the recipe declared in
that file, if any). With `share`, the declaration joins the
*workspace-wide* catalog visible to every other `.forage` file in
the workspace. `input` and `secret` are recipe-local by nature —
`share` does not apply.

Workspace-wide name collisions among `share`d declarations are a
validator error. Inside a single file, a file-scoped declaration
overrides a same-name `share`d declaration from elsewhere.

## Function declarations

```
fn_decl              := 'share'? 'fn' Ident '(' param_list? ')'
                        '{' fn_body '}'

param_list           := DollarVar ( ',' DollarVar )*

fn_body              := let_binding* extraction

let_binding          := 'let' DollarVar '=' extraction ( ';' )?
```

`$input` and `$secret` are reserved roots and cannot be used as
parameter or let-binding names. `let` is fn-body-only — not legal
inside step bodies, emit bindings, or top-level expressions. A fn
body always ends in a single trailing expression that is the
return value.

## Statements

```
statement            := step | emit | for_loop

emit                 := 'emit' TypeName '{' emit_binding_list '}' bind_suffix?

emit_binding_list    := ( emit_binding ( ';' | ',' )? )*

emit_binding         := field_name '←' extraction

bind_suffix          := 'as' DollarVar

for_loop             := 'for' DollarVar 'in' extraction
                        '{' statement* '}'

step                 := 'step' field_name '{' step_field* '}'

step_field           := 'method'  ':' STRING
                      | 'url'     ':' STRING
                      | 'headers' '{' header_kv_list '}'
                      | 'body' '.' body_kind '{' body_contents '}'
                      | 'paginate' pagination_block
                      | 'extract' '.' 'regex' '{' regex_extract_body '}'

header_kv_list       := ( STRING ':' STRING ( ',' | ';' )? )*

body_kind            := 'json' | 'form' | 'raw'

body_contents        := json_body_kvs       (when body_kind = 'json')
                      | form_body_kvs       (when body_kind = 'form')
                      | STRING              (when body_kind = 'raw')

json_body_kvs        := ( body_key ':' body_value ( ',' | ';' )? )*

form_body_kvs        := ( STRING ':' body_value ( ',' | ';' )? )*

body_key             := Ident | Keyword | STRING

body_value           := '{' json_body_kvs '}'
                      | '[' ( body_value ( ',' body_value )* )? ']'
                      | 'case' path 'of' '{' body_case_arms '}'
                      | path
                      | literal
                      | STRING                          (template if contains '{')

body_case_arms       := ( case_label '→' body_value ( ',' | ';' )? )*

regex_extract_body   := ( 'pattern' ':' STRING
                        | 'groups'  ':' '[' STRING* ']' )
                        ( ',' | ';' )?
```

A `step` requires `method` and `url` (validated post-parse). `url`
strings are templates — `{...}` segments are re-lexed as
`extraction`s; literal text is preserved. The same applies to header
values, `body.json` values typed as `STRING`, and `body.raw`.

An `emit … as $v` introduces `$v` into the enclosing lexical scope
with type `Ref<TypeName>`. Subsequent statements in the same scope
can reference `$v`.

## Pagination

HTTP-side pagination on a `step`:

```
pagination_block     := 'paginate' pagination_strategy '{' paginate_field* '}'

pagination_strategy  := 'pageWithTotal' | 'untilEmpty' | 'cursor'

paginate_field       := 'items'           ':' path
                      | 'total'           ':' path
                      | 'cursorPath'      ':' path
                      | 'pageParam'       ':' STRING
                      | 'cursorParam'     ':' STRING
                      | 'pageSize'        ':' INT
                      | 'pageZeroIndexed' ':' BOOL
```

Required-field validation happens post-parse based on the strategy:

- `pageWithTotal` needs `items`, `total`, `pageParam`.
- `untilEmpty` needs `items`, `pageParam`.
- `cursor` needs `items`, `cursorPath`, `cursorParam`.

## Auth

```
auth_block           := 'auth' '.' auth_strategy

auth_strategy        := static_header | html_prime | session

static_header        := 'staticHeader' '{' static_header_fields '}'

static_header_fields := ( ( 'name'  ':' STRING
                          | 'value' ':' STRING ) ( ',' | ';' )? )*

html_prime           := 'htmlPrime' '{' html_prime_fields '}'

html_prime_fields    := ( ( ( 'step' | 'stepName' ) ':' ( Ident | STRING )
                          | 'nonceVar'   ':' STRING
                          | 'ajaxUrlVar' ':' STRING ) ( ',' | ';' )? )*

session              := 'session' '.' session_variant '{' session_fields '}'

session_variant      := 'formLogin' | 'bearerLogin' | 'cookiePersist'

session_fields       := ( session_field ( ',' | ';' )? )*

session_field        := 'url'              ':' STRING
                      | 'method'           ':' STRING
                      | 'body' '.' body_kind '{' body_contents '}'
                      | 'tokenPath'        ':' path
                      | 'headerName'       ':' STRING
                      | 'headerPrefix'     ':' STRING
                      | 'sourcePath'       ':' STRING
                      | 'format'           ':' ( STRING | Ident | Keyword )
                      | 'captureCookies'   ':' BOOL
                      | 'maxReauthRetries' ':' INT
                      | 'cache'            ':' INT
                      | 'cacheEncrypted'   ':' BOOL
                      | 'requiresMFA'      ':' BOOL
                      | 'mfaFieldName'     ':' STRING
```

Required-field validation per session variant is post-parse;
`formLogin` and `bearerLogin` require `url`; `bearerLogin` requires
`tokenPath`; `cookiePersist` requires `sourcePath`.

## Browser block

```
browser_block        := 'browser' '{' browser_field* '}'

browser_field        := 'initialURL'    ':' STRING
                      | 'observe'       ':' STRING
                      | 'ageGate' '.' 'autoFill' '{' age_gate_fields '}'
                      | 'dismissals' '{' dismissals_fields '}'
                      | 'warmupClicks' ':' '[' STRING* ']'
                      | 'paginate' browser_paginate
                      | 'captures' '.' ( 'match' | 'document' )
                                       '{' capture_body '}'
                      | 'interactive' interactive_block

age_gate_fields      := ( ( 'dob'                ':' DATE
                          | 'reloadAfter'        ':' BOOL
                          | 'reloadAfterSubmit'  ':' BOOL ) ( ',' | ';' )? )*

dismissals_fields    := ( ( 'maxIterations' ':' INT
                          | 'extraLabels'   ':' '[' STRING* ']' )
                          ( ',' | ';' )? )*

browser_paginate     := 'paginate' ( '.' | 'browserPaginate' '.' )
                        ( 'scroll' | 'replay' )
                        '{' browser_paginate_fields '}'

browser_paginate_fields :=
                        ( ( 'until' ':' 'noProgressFor' '(' INT ')'
                          | 'maxIterations'  ':' INT
                          | 'iterationDelay' ':' ( FLOAT | INT )
                          | 'seedFilter'     ':' STRING )
                          ( ',' | ';' )? )*

capture_body         := match_capture_body | document_capture_body

match_capture_body   := 'urlPattern' ':' STRING
                        'for' DollarVar 'in' extraction
                        '{' statement* '}'

document_capture_body := 'for' DollarVar 'in' extraction
                         '{' statement* '}'

interactive_block    := 'interactive' '{' interactive_fields '}'

interactive_fields   := ( ( 'bootstrapURL'         ':' STRING
                          | 'cookieDomains'        ':' '[' STRING* ']'
                          | 'sessionExpiredPattern' ':' STRING )
                          ( ',' | ';' )? )*
```

Browser blocks require `initialURL`, `observe`, and `paginate`; the
rest are optional. Only one `captures.document` is allowed per
browser block; `captures.match` may repeat.

## Expectations

```
expect_block         := 'expect' '{'
                        'records' '.' 'where' '('
                          'typeName' '==' STRING
                        ')' '.' 'count' cmp_op INT
                        '}'

cmp_op               := '>' '='?           (Gt or Ge)
                      | '<' '='?           (Lt or Le)
                      | '=' '='            (Eq)
                      | '!' '='            (Ne)
```

Only the `records.where(typeName == "...").count <op> N` form is
supported today. Any other shape is a parse error.

## Expressions

The expression grammar drives emit field bindings, step body values,
template interpolations, fn bodies, and case branches. Identical
across all those contexts.

```
extraction           := pipe

pipe                 := additive ( '|' transform_call )*

additive             := multiplicative ( ( '+' | '-' ) multiplicative )*

multiplicative       := unary ( ( '*' | '/' | '%' ) unary )*

unary                := '-' unary
                      | postfix

postfix              := primary ( '[' extraction ']' )*       (only on Call/StructLiteral)

primary              := 'case' path 'of' '{' case_arms '}'
                      | struct_literal
                      | regex_literal
                      | '(' extraction ')'
                      | call
                      | path
                      | literal

struct_literal       := '{' ( struct_field ( ',' | ';' )? )* '}'

struct_field         := field_name ':' extraction

regex_literal        := REGEX

call                 := Ident '(' arg_list? ')'

arg_list             := extraction ( ',' extraction )*

transform_call       := ( Ident | Keyword ) ( '(' arg_list? ')' )?

case_arms            := ( case_arm ( ',' | ';' )? )*

case_arm             := case_label '→' extraction

case_label           := Ident | TypeName | Keyword
                      | BOOL | NULL | INT | STRING

literal              := STRING | INT | FLOAT | BOOL | NULL

path                 := path_head path_step*

path_head            := '$'                              (Current)
                      | '$input'                         (Input)
                      | '$secret' '.' Ident              (Secret access)
                      | DollarVar                        (loop var, emit binding, fn param)

path_step            := '.'  path_field                  (field access)
                      | '?.' path_field                  (optional-chained access)
                      | '[' INT ']'                      (literal index, null-tolerant)
                      | '[*]'                            (iterate / map)

path_field           := Ident | TypeName | Keyword
```

Precedence is low-to-high in the rule order above. `|` (pipe) is
lowest so `$x * 28 | toString` reads as `($x * 28) | toString`.

The path-level `[N]` postfix only accepts a literal integer and is
null-tolerant; the expression-level `[expr]` postfix accepts any
expression but is strict (out-of-bounds raises an error). They're
distinct productions — the path form rides under `path_step`, the
expression form under `postfix`.

A `STRING` that contains a `{` becomes a template — every `{...}`
segment is re-lexed and parsed as an `extraction`. Templates appear
wherever a string literal appears in step bodies, URLs, headers, raw
bodies, and string-typed emit bindings.

## Reserved words

A complete list of keywords lives in `KEYWORDS` at
`crates/forage-core/src/parse/token.rs`. The parser uses two
categories:

- **Reserved at top level** as statement / declaration heads or
  modifiers: `recipe`, `engine`, `share`, `type`, `enum`, `fn`,
  `input`, `output`, `secret`, `auth`, `browser`, `step`, `for`,
  `in`, `emit`, `as`, `case`, `of`, `let`, `expect`.
- **Reserved inside structured forms** as field keys:
  `method`, `url`, `headers`, `body`, `json`, `form`, `raw`,
  `extract`, `regex`, `groups`, `paginate`, `pageWithTotal`,
  `untilEmpty`, `cursor`, `items`, `total`, `pageParam`, `pageSize`,
  `cursorPath`, `cursorParam`, `pageZeroIndexed`, …
  Many of these can appear as field / path / call names in
  expression position (`field_name`, `path_field`, transform name) —
  the parser accepts `Keyword` there in addition to `Ident`.

The lexer's `TYPE_KEYWORDS` (`String`, `Int`, `Double`, `Bool`) are
reserved as type-position keywords. `Ref` is a contextual keyword
in type position only.

## Notes

- **At most one recipe per file.** A second `recipe` header is a
  validator error. Files without a header are pure declarations
  files; they're valid as long as they don't contain recipe-context
  forms (auth / browser / expect / statement).
- **Order is free.** Top-level forms — the recipe header, types,
  enums, inputs, secrets, fn declarations, auth, browser config,
  expectations, and statements — can intermix at the file root in
  any order. The parser collects each kind into its slot on the
  `ForageFile` AST; ordering is not load-bearing.
- **`share` visibility.** `share type Foo { … }` makes `Foo`
  workspace-visible; bare `type Foo { … }` is file-scoped.
  Workspace-wide `share`d-name collisions are a validator error; a
  file-scoped declaration overrides a same-name `share`d
  declaration when both reach the same recipe's catalog.
- **No filesystem-position semantics.** File location within a
  workspace is organizational, not load-bearing. A workspace is a
  flat directory of `.forage` source files plus (optionally)
  `_fixtures/`, `_snapshots/` data dirs and the hidden `.forage/`
  runtime store. The `TypeCatalog` walks every `.forage` file in
  the workspace and pulls in `share`d declarations regardless of
  where they sit.
- **Field-position keywords.** `name`, `value`, `headers`, etc. are
  keywords reserved at structured-form sites but accepted as field
  names elsewhere. The parser's `expect_field_name` and
  `expect_case_label` accept both `Ident` and `Keyword`.
- **`$page` is reserved at runtime** (engine-injected loop var) but
  not at parse time. The validator (`ReservedParam`) rejects fn
  declarations that name `$page`.
- **Greenfield, not stable.** The grammar evolves; this document is
  re-generated whenever the parser changes. Production rules in the
  parser have doc comments tracking each non-terminal — keep them in
  sync.
