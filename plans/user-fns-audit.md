# user-fns audit

Audited `user-fns` branch (`d7a5e6f`, 7 commits) against
`plans/user-defined-functions.md`. Branch rebased onto current `origin/main`
(already in lockstep; no merge work needed).

## Acceptance commands

- PASS `cargo check --workspace`.
- PASS `cargo test --workspace`. Notable suite counts (matching PE claims):
  forage-core lib tests 88 passed, `parser_smoke` 19 passed, `eval_smoke` 6
  passed, `ast_roundtrip` 7 passed, `shared_recipes` 1 passed, engine/daemon/
  studio/keychain/etc. all green.
- PASS `npm test` in `hub-site/forage-ts`: 9 files / 68 tests.
- PASS `npm test` in `apps/studio/ui`: 2 files / 13 tests (after
  pre-existing missing `node_modules` was hydrated by `npm install`; not a
  regression from this PR).
- PASS `rg "BUILTIN_TRANSFORMS\b" crates/forage-core` returns the same
  three references in `validate/mod.rs` and lists the identical 30-name
  set as `eval::transforms::build_default`. No built-ins removed.

## Findings

### 🔴 Critical

**1. Zero-parameter user fns are uncallable at runtime.**
`crates/forage-core/src/eval/mod.rs:237-244` computes
`provided = args.len() + 1` unconditionally — head plus explicit args.
For a zero-param fn called either via direct call (`answer()`) or pipe
(`$x | answer`), `expected = 0` but `provided = 1`, so `apply_user_fn`
returns `EvalError::Generic("function 'answer' expects 0 arguments, got 1")`.

The parser accepts zero-param fns (`fn_decl_zero_params_parses` in
`crates/forage-core/tests/parser_smoke.rs:362`); the validator allows
calling them with zero args (`apply_direct_call` arity branch in
`validate/mod.rs:681-708` accepts `declared == call_arity == 0`); the docs
advertise them (`docs/src/lang/functions.md:29`: "Zero parameters is
allowed: `fn now() { … }`"). All paths green-light a feature the eval
breaks.

Same bug in the TS port at `hub-site/forage-ts/src/extraction.ts:179-186`
(`provided = rest.length + 1`). Parity, but parity on a broken feature.

The eval check disagrees with the validator: validator counts head only
when there *is* a pipe head (pipe form: `c.args.len() + 1`; direct form:
`args.len()` — see `validate/mod.rs:645,668`). Eval should mirror that —
for direct calls it should count `resolved.len()`, not synthesize a head.

Fix: in `apply_direct_call`, compute arity against `resolved.len()`
directly and only inject a synthetic head when the user fn has ≥1
parameter. In `apply_pipe_call`, the head is real; arity = `args.len()
+ 1` stays correct. (Equivalently: make `apply_user_fn` take a `head:
Option<EvalValue>` and let callers pass `None` when there's no implicit
head.)

No fixture exercises a zero-param user-fn call at runtime; the bug is
invisible until a recipe author writes one. Plan example
`fn now() { … }` would die at runtime today.

### 🟡 Significant

**2. Plan-specified `EvalError::FnArityMismatch` variant is missing.**
Plan (`user-defined-functions.md:196-198`): "If a runtime mismatch slips
through, surface `EvalError::FnArityMismatch` with a clear message."
Implementation uses `EvalError::Generic(format!(...))` at
`crates/forage-core/src/eval/mod.rs:240-244`. The error variant
enumeration in `crates/forage-core/src/eval/error.rs` has no
`FnArityMismatch` case. Functional but typed-error discipline is lost —
callers can't pattern-match the case.

**3. No parser-side rejection test for `fn foo($input)` / `fn foo($secret)`.**
PE deviation #4 claimed the lexer's distinct `DollarInput`/`DollarSecret`
tokens drive parser-side rejection and "leave `$page` as the only
`ReservedParam`." The mechanism works — `parse_fn_decl` at
`crates/forage-core/src/parse/parser.rs:382-394` accepts only
`Token::DollarVar` and falls through `unexpected("parameter ($name)")`
on `DollarInput`/`DollarSecret`. But no test pins this: the existing
`fn_decl_rejects_non_dollar_param` uses bare `x`, not `$input`/`$secret`.
A regression that lets `$input` slip through as a param token would
make `ReservedParam` validation a dead branch and no test would catch
it. Add a parser test asserting `fn foo($input) { … }` errors at the
parser layer.

The error message is generic (`expected parameter ($name), got '$input'`)
— it doesn't tell the recipe author *why* `$input` is rejected. Not a
defect per the plan (which is silent on message text), but worth noting
for friendliness later.

**4. TS-port shared-recipe coverage works but is structural-only.**
`hub-site/forage-ts/test/shared.test.ts:130-132` checks
`recipe.functions` length against `functionCount`, and the
`Tests/shared-recipes/expected.json` entry for `09-user-functions.forage`
declares `functionCount: 2`. The TS port parses + validates the recipe
but does not *run* it against an expected snapshot — the
`expected.json` shape stops at structural summaries
(`typeCount`, `fieldNames`, `validation`). The Rust `shared_recipes.rs`
harness has the same shape. Plan's "What done looks like" says: "The
`09-user-functions.forage` shared-recipes test produces the expected
snapshot in both the Rust engine and the TS port." There's no snapshot
in expected.json; cross-implementation eval parity is not enforced by
the shared harness, only by `eval_smoke` + `test/user-fns.test.ts`
running side-by-side fixtures. Drift between Rust eval and TS eval
won't be caught by `shared-recipes`. Either extend the JSON shape to
include a `runSnapshot` or accept that shared coverage is parser/
validator only and note it in the plan.

### 🔵 Minor

**5. `DuplicateParam` docstring is one line.**
`crates/forage-core/src/validate/mod.rs:84-85`: "A `fn` declaration
lists the same `$param` name twice." Other codes go a sentence longer
explaining recipe-author consequences (`DuplicateFn`: "calls would be
ambiguous — only the first one would resolve"). Cosmetic.

**6. `FnDecl.span` uses `#[serde(default)]`.**
`crates/forage-core/src/ast/recipe.rs:87-88`. Plan emphasizes no
`#[serde(default)]` on `functions`; nothing said about the inner
span. Existing convention (look at `RecipeType.span`, etc.) uses
`#[serde(default)]` on spans because they're position metadata,
not semantic content. Consistent with codebase, not a defect — note
only because it could surprise on a strict reading of plan §"Style
/ discipline."

**7. Doc example syntax matches impl.**
`docs/src/lang/functions.md:12-17`'s `case $name of { … }` parses
cleanly through `parser.rs:602-621` (separators between branches are
optional). The plan's example dropped the `{ }` braces (`else`-less)
and used `else $name`; doc fixed both to match the actual grammar.
Good. The doc `parseGramsFromOunces` worked example promised in plan
§ Docs is not present, but the plan itself hedged ("when math
primitives exist; for now, use built-in transforms only") and the
example `shout` covers the same ground.

### 💭 Questions

None — the design intent is clear from the plan and the code is
straightforward to follow.

## Cross-cutting checks

- AST `FnDecl` shape matches plan: `name`, `params`, `body`, `span`
  (`recipe.rs:80-89`). `Recipe.functions: Vec<FnDecl>`
  (`recipe.rs:63`) with NO `#[serde(default)]` — greenfield discipline
  upheld (all sibling fields keep their existing `#[serde(default)]`;
  `functions` is the lone exception, per plan §"Style / discipline").
- Validator scope rules: closed scope built in `check_user_fns`
  (`validate/mod.rs:314-330`). Tests exercise the three plan-named
  cases (`for_loop_var_not_visible_in_fn_body`,
  `as_binding_not_visible_in_fn_body`, `secret_and_input_visible_in_fn_body`).
  Each asserts the right code variant, not just any Err.
- Direct recursion: `direct_recursion_emits_warning`
  (`validate/mod.rs:1502-1519`) asserts both `!report.has_errors()` and
  `RecursiveFunction` warning of severity `Severity::Warning`. Matches
  plan.
- Arity asymmetry between `apply_pipe_call` / `apply_direct_call`
  documented inline at `eval/mod.rs:171-198` and again at
  `extraction.ts:151-154`. The TS-side comment is briefer but conveys
  the asymmetry. Exercised by `user_fn_called_via_pipe_passes_head_as_first_param`
  + `user_fn_called_via_direct_call_passes_all_args` (Rust) and the
  pipe-form runner test in `test/user-fns.test.ts`.
- `TransformRegistry::with_user_fns` clones the built-in HashMap by
  value (`transforms.rs:52`). PE flagged the clone cost; the call
  fires once per `Engine::run`, not per record — amortized over the
  whole scrape, not a hot path.
- No `#[allow(dead_code)]` introduced in the changed surface. Two
  `#[allow(dead_code)]` survive in `tests/shared_recipes.rs` (pre-existing,
  outside this PR). No `unwrap_or_else(|| "default")` masking. No
  `--no-verify`. No out-of-scope drift — grepped for `Operator::`,
  `Regex` (in eval, ignoring the engine's existing
  `apply_regex_extract`), `LetBinding`, etc. None added.

## Verdict

**Request Changes.** The zero-parameter direct-call bug (Finding 1)
is a silent runtime failure on a feature surface the docs, parser, and
validator all advertise. Plan example `fn now() { … }` dies at the
first call site today. Both Rust and TS ports share the bug. Add the
missing arity case (a single-line conditional) plus a runtime test in
`eval_smoke.rs` + `user-fns.test.ts`, then ff-merge.

Finding 2 (typed `FnArityMismatch` variant) is recommended in the same
PR — the variant exists in the plan and the current `Generic` carve-out
is a small, contained miss. Finding 3 (parser rejection test for
`$input`/`$secret`) is also recommended in the same PR; it pins down a
deviation the PE explicitly called out.

Findings 4–7 can land separately or be deferred.
