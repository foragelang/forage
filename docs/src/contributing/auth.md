# Adding an auth flavor

Auth flavors are recipe-level keywords (`auth.staticHeader`,
`auth.htmlPrime`, `auth.session.formLogin`, …) the engine consumes
before any data step runs. Adding a new flavor touches four layers.

## 1. AST

In `crates/forage-core/src/ast/auth.rs`, add a variant to
`AuthStrategy` (or, for a session variant, a variant to `SessionKind`).
Keep the struct shape parallel to the existing ones:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthStrategy {
    StaticHeader { name: String, value: Template },
    HtmlPrime    { step_name: String, captured_vars: Vec<HtmlPrimeVar> },
    Session(SessionAuth),
    // New variant here:
    AwsSigv4 { region: String, service: String, access_key: PathExpr, secret_key: PathExpr },
}
```

If it's a session-style flavor (per-run resolution, optional cache),
add a new `SessionKind` variant instead so it inherits cache /
re-auth / MFA semantics for free.

## 2. Parser

In `crates/forage-core/src/parse/parser.rs::parse_auth`, add a
`match` arm matching the new keyword. Reuse the existing field-bag
parsing loop where possible; bail with `self.generic("…")` for
required-field-missing errors so they surface in the diagnostic.

Add the new keywords to `crates/forage-core/src/parse/token.rs::KEYWORDS`
so the lexer recognizes them.

## 3. Runtime

In `crates/forage-http/src/auth.rs`:

- Extend `run_session_login` (if it's a session variant) or
  `apply_request_headers` (if it's a per-request modifier) to handle
  the new variant.
- Reuse `forage_http::body::render_body` if the new flavor sends a
  request body, `forage_keychain` for any persisted state, and the
  existing retry loop in `client.rs`.

If the flavor binds vars into scope (htmlPrime-style), do it through
the existing `scope.bind` machinery so subsequent steps see the values
via `{$varName}` templates.

## 4. Validation

`crates/forage-core/src/validate/mod.rs` needs:

- A reference-resolution rule (the validator should flag dangling
  references the new flavor introduces; e.g. if it references a step
  by name, check that step exists).
- A `ValidationCode` entry for any auth-specific error class.
- A `BUILTIN_TRANSFORMS`-style allowlist if it introduces a new pseudo-
  transform.

## 5. LSP + IDE

If the new keywords matter for completion / hover:

- Add them to `crates/forage-lsp/src/server.rs::KEYWORDS`.
- Add them to `apps/studio/ui/src/lib/monaco-forage.ts::KEYWORDS`.

The LSP rebuilds diagnostics on the next document change; recipe
authors get the new flavor's affordances immediately.

## 6. Docs

- A short section under `docs/src/lang/auth.md` describing the new
  flavor with a minimal example.
- A row in the auth-flavors table.
- A note in `docs/src/runtime/sessions.md` if the flavor carries
  persistent state.

## Tests

- AST: a JSON round-trip test (parse → serialize → parse).
- Parser: a `tests/parser_smoke.rs` case with the new keyword.
- Validator: a positive case (clean recipe) + negative case (the new
  diagnostic fires).
- HTTP engine: a `wiremock`-driven integration test that confirms the
  request shape the new flavor produces.

The shared-recipes harness pulls test vectors from `Tests/shared-recipes/`;
if the new flavor warrants a canonical example for the TS + Rust
implementations to agree on, add `XX-auth-<flavor>.forage` + the
matching descriptor in `expected.json`.
