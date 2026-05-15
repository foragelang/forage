//! Forage parser — token stream → `Recipe`.
//!
//! Currently a hand-rolled recursive-descent parser over the token vector.
//! Wrapped with a public `parse(source)` that runs the lexer first and
//! aggregates lex + parse errors with spans.
//!
//! The grammar is documented inline near each `parse_*` function and
//! tracks `Sources/Forage/Parser/Parser.swift` closely; tests in
//! `tests/parser_recipes.rs` exercise every in-tree recipe.

use thiserror::Error;

use crate::ast::*;
use crate::parse::lexer::{LexError, lex};
use crate::parse::token::Token;

#[derive(Debug, Clone, Error, PartialEq)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected token at {span:?}: expected {expected}, found {found}")]
    UnexpectedToken {
        span: std::ops::Range<usize>,
        expected: String,
        found: String,
    },
    #[error("unexpected end of input; expected {expected}")]
    UnexpectedEof { expected: String },
    #[error("{message} at {span:?}")]
    Generic {
        span: std::ops::Range<usize>,
        message: String,
    },
    #[error("invalid regex at {span:?}: {message}")]
    InvalidRegex {
        span: std::ops::Range<usize>,
        message: String,
    },
    #[error("invalid regex flag '{flag}' at {span:?}")]
    InvalidRegexFlag {
        span: std::ops::Range<usize>,
        flag: char,
    },
}

/// Top-level entry: lex + parse a `.forage` file. The grammar is flat —
/// a file is a sequence of top-level forms (`recipe` header, `type`,
/// `enum`, `input`, `secret`, `fn`, `auth`, `browser`, `expect`,
/// statements). The parser collects each form into the matching slot on
/// `ForageFile` regardless of source order.
///
/// Semantic constraints — "at most one recipe header", "recipe-context
/// forms require a header", "no duplicate workspace-shared
/// declarations" — live in the validator, not the parser.
pub fn parse(source: &str) -> Result<ForageFile, ParseError> {
    let toks = lex(source)?;
    let mut p = Parser::new(toks, source.len());
    p.parse_forage_file()
}

struct Parser {
    toks: Vec<(Token, std::ops::Range<usize>)>,
    pos: usize,
    eof_pos: usize,
}

impl Parser {
    fn new(toks: Vec<(Token, std::ops::Range<usize>)>, eof_pos: usize) -> Self {
        Self {
            toks,
            pos: 0,
            eof_pos,
        }
    }

    // --- low-level helpers -------------------------------------------------

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.toks.get(self.pos + offset).map(|(t, _)| t)
    }

    fn current_span(&self) -> std::ops::Range<usize> {
        self.toks
            .get(self.pos)
            .map(|(_, s)| s.clone())
            .unwrap_or(self.eof_pos..self.eof_pos)
    }

    /// Byte offset where the *previous* token (the one just consumed) ends.
    /// Used to close out a span for a parsed construct: capture
    /// `current_span().start` at entry, consume the construct's tokens, then
    /// `prev_end()` is the end of the closing brace / last consumed token.
    fn prev_end(&self) -> usize {
        if self.pos == 0 {
            return 0;
        }
        self.toks
            .get(self.pos - 1)
            .map(|(_, s)| s.end)
            .unwrap_or(self.eof_pos)
    }

    /// Build a span from `start` (captured at entry) to the end of the
    /// previously-consumed token.
    fn span_to_here(&self, start: usize) -> std::ops::Range<usize> {
        start..self.prev_end()
    }

    fn bump(&mut self) -> Option<&(Token, std::ops::Range<usize>)> {
        let r = self.toks.get(self.pos);
        if r.is_some() {
            self.pos += 1;
        }
        r
    }

    fn eat_punct(&mut self, t: &Token) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, t: &Token) -> Result<(), ParseError> {
        if self.eat_punct(t) {
            Ok(())
        } else {
            Err(self.unexpected(t.describe()))
        }
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        match self.peek() {
            Some(Token::Keyword(k)) if k == kw => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), ParseError> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            Err(self.unexpected(&format!("'{kw}'")))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Token::Ident(s)) => {
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.unexpected("identifier")),
        }
    }

    fn expect_typename(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Token::TypeName(s)) => {
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.unexpected("type name")),
        }
    }

    fn expect_string(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Token::Str(s)) => {
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.unexpected("string literal")),
        }
    }

    fn expect_int(&mut self) -> Result<i64, ParseError> {
        // Accept `-N` as a single integer here. The lexer always emits
        // `Dash` for `-`, but configuration fields (`pageSize: -1`,
        // `[-1]` for tail indexing) want a signed literal in one shot;
        // wedging an `Unary` AST node in for these would propagate the
        // type through every config struct that holds an `i64`.
        let neg = self.eat_punct(&Token::Dash);
        match self.peek().cloned() {
            Some(Token::Int(n)) => {
                self.pos += 1;
                Ok(if neg { -n } else { n })
            }
            _ => Err(self.unexpected("integer literal")),
        }
    }

    fn unexpected(&self, expected: &str) -> ParseError {
        match self.peek() {
            None => ParseError::UnexpectedEof {
                expected: expected.into(),
            },
            Some(t) => ParseError::UnexpectedToken {
                span: self.current_span(),
                expected: expected.into(),
                found: t.describe().into(),
            },
        }
    }

    // --- grammar -----------------------------------------------------------

    /// forage_file := top_level_form*
    /// top_level_form := recipe_header | type_decl | enum_decl | input_decl
    ///                 | secret_decl | fn_decl | auth_block | browser_block
    ///                 | expect_block | statement
    ///
    /// One file format. Every form lands in its slot on `ForageFile`
    /// regardless of source order. Semantic rules — at most one recipe
    /// header, recipe-context forms require a header, no duplicate
    /// shared decls across the workspace — are the validator's job.
    fn parse_forage_file(&mut self) -> Result<ForageFile, ParseError> {
        let mut recipe_headers: Vec<RecipeHeader> = Vec::new();
        let mut types: Vec<RecipeType> = Vec::new();
        let mut enums: Vec<RecipeEnum> = Vec::new();
        let mut inputs: Vec<InputDecl> = Vec::new();
        let mut secrets: Vec<String> = Vec::new();
        let mut functions: Vec<FnDecl> = Vec::new();
        let mut auth: Option<AuthStrategy> = None;
        let mut browser: Option<BrowserConfig> = None;
        let mut body: Vec<Statement> = Vec::new();
        let mut expectations: Vec<Expectation> = Vec::new();

        while self.peek().is_some() {
            match self.peek().cloned() {
                Some(Token::Keyword(k)) => match k.as_str() {
                    "recipe" => {
                        // Every header is kept; the validator's
                        // `DuplicateRecipeHeader` rule fires on the
                        // second one and onwards.
                        recipe_headers.push(self.parse_recipe_header()?);
                    }
                    "share" => {
                        // `share` is an optional visibility prefix on
                        // type / enum / fn. Anything else after it is a
                        // parse error.
                        self.bump();
                        match self.peek().cloned() {
                            Some(Token::Keyword(k2)) if k2 == "type" => {
                                types.push(self.parse_type_decl_shared(true)?);
                            }
                            Some(Token::Keyword(k2)) if k2 == "enum" => {
                                enums.push(self.parse_enum_decl_shared(true)?);
                            }
                            Some(Token::Keyword(k2)) if k2 == "fn" => {
                                functions.push(self.parse_fn_decl_shared(true)?);
                            }
                            _ => {
                                return Err(self.unexpected(
                                    "'type', 'enum', or 'fn' after 'share'",
                                ));
                            }
                        }
                    }
                    "type" => types.push(self.parse_type_decl_shared(false)?),
                    "enum" => enums.push(self.parse_enum_decl_shared(false)?),
                    "input" => inputs.push(self.parse_input_decl()?),
                    "secret" => {
                        self.bump();
                        let s = self.expect_ident()?;
                        secrets.push(s);
                    }
                    "auth" => {
                        if auth.is_some() {
                            return Err(self.generic("duplicate auth block"));
                        }
                        auth = Some(self.parse_auth()?);
                    }
                    "browser" => {
                        if browser.is_some() {
                            return Err(self.generic("duplicate browser block"));
                        }
                        browser = Some(self.parse_browser_block()?);
                    }
                    "expect" => expectations.push(self.parse_expect_block()?),
                    "fn" => functions.push(self.parse_fn_decl_shared(false)?),
                    "step" | "for" | "emit" => {
                        body.push(self.parse_statement()?);
                    }
                    other => return Err(self.generic(&format!("unexpected keyword '{other}'"))),
                },
                Some(other) => {
                    return Err(self.generic(&format!(
                        "unexpected token at top level: {}",
                        other.describe()
                    )));
                }
                None => break,
            }
        }

        Ok(ForageFile {
            recipe_headers,
            types,
            enums,
            inputs,
            secrets,
            functions,
            auth,
            browser,
            body,
            expectations,
        })
    }

    /// recipe_header := 'recipe' STRING 'engine' engine_kind
    fn parse_recipe_header(&mut self) -> Result<RecipeHeader, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("recipe")?;
        let name = self.expect_string()?;
        self.expect_keyword("engine")?;
        let engine_kind = self.parse_engine_kind()?;
        Ok(RecipeHeader {
            name,
            engine_kind,
            span: self.span_to_here(start),
        })
    }

    /// fn_decl := 'fn' ident '(' ($ident (',' $ident)*)? ')'
    ///            '{' (let_binding)* trailing_expr '}'
    /// let_binding := 'let' '$' ident '=' extraction
    ///
    /// The body is a sequence of let-bindings followed by exactly one
    /// trailing expression — the return value. Let-bindings are
    /// fn-body-only (not legal in step bodies, emit bindings, or
    /// top-level expressions); the rest of the language stays
    /// declarative.
    ///
    /// `$input` / `$secret` are reserved roots — the lexer emits them as
    /// distinct tokens (not `DollarVar`), so they're rejected here with
    /// a recipe-author message instead of the generic
    /// `expected parameter ($name)` fallback. `$page` is engine-injected
    /// and is rejected later by the validator (`ReservedParam`).
    /// fn_decl := 'share'? 'fn' Ident '(' param_list? ')' '{' fn_body '}'
    fn parse_fn_decl_shared(&mut self, shared: bool) -> Result<FnDecl, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("fn")?;
        let name = self.expect_ident()?;
        self.expect_punct(&Token::LParen)?;
        let mut params: Vec<String> = Vec::new();
        if !self.eat_punct(&Token::RParen) {
            loop {
                match self.peek().cloned() {
                    Some(Token::DollarVar(s)) => {
                        self.bump();
                        params.push(s);
                    }
                    Some(Token::DollarInput) | Some(Token::DollarSecret) => {
                        let span = self.current_span();
                        let found = self.peek().unwrap().describe();
                        return Err(ParseError::Generic {
                            span,
                            message: format!(
                                "{found} is a reserved root and cannot be a fn parameter; pick another name",
                            ),
                        });
                    }
                    _ => return Err(self.unexpected("parameter ($name)")),
                }
                if !self.eat_punct(&Token::Comma) {
                    break;
                }
            }
            self.expect_punct(&Token::RParen)?;
        }
        self.expect_punct(&Token::LBrace)?;
        let body = self.parse_fn_body()?;
        self.expect_punct(&Token::RBrace)?;
        Ok(FnDecl {
            name,
            params,
            body,
            shared,
            span: self.span_to_here(start),
        })
    }

    /// `(let_binding)* trailing_expr` — the inside of a fn body. The
    /// trailing expression isn't a separate non-terminal; we keep
    /// parsing let-bindings while the next token is `let`, then read
    /// one final extraction. Authors who write zero `let`s get a
    /// single-expression body, identical to the pre-let shape.
    fn parse_fn_body(&mut self) -> Result<FnBody, ParseError> {
        let mut bindings: Vec<LetBinding> = Vec::new();
        while self.peek_is_let() {
            self.bump(); // `let` keyword
            let name = match self.peek().cloned() {
                Some(Token::DollarVar(s)) => {
                    self.bump();
                    s
                }
                Some(Token::DollarInput) | Some(Token::DollarSecret) => {
                    let span = self.current_span();
                    let found = self.peek().unwrap().describe();
                    return Err(ParseError::Generic {
                        span,
                        message: format!(
                            "{found} is a reserved root and cannot be a let-binding name",
                        ),
                    });
                }
                _ => return Err(self.unexpected("let binding name ($name)")),
            };
            self.expect_punct(&Token::Equal)?;
            let value = self.parse_extraction()?;
            bindings.push(LetBinding { name, value });
            // Optional separator — recipe authors often write a `;` or
            // newline after a binding; allow both, neither is required.
            let _ = self.eat_punct(&Token::Semicolon);
        }
        let result = self.parse_extraction()?;
        Ok(FnBody { bindings, result })
    }

    fn peek_is_let(&self) -> bool {
        matches!(self.peek(), Some(Token::Keyword(k)) if k == "let")
    }

    fn parse_engine_kind(&mut self) -> Result<EngineKind, ParseError> {
        match self.peek().cloned() {
            Some(Token::Keyword(k)) if k == "http" => {
                self.bump();
                Ok(EngineKind::Http)
            }
            Some(Token::Keyword(k)) if k == "browser" => {
                self.bump();
                Ok(EngineKind::Browser)
            }
            _ => Err(self.unexpected("'http' or 'browser'")),
        }
    }

    // --- type / enum / input ----------------------------------------------

    /// type_decl := 'share'? 'type' TypeName '{' field (';'|',')? ... '}'
    ///
    /// `share` consumption happens in `parse_forage_file`; this helper
    /// just receives the flag and the head `type` keyword still in the
    /// stream. The span covers the `type` keyword through the closing
    /// brace (the `share` prefix sits outside the recorded span).
    fn parse_type_decl_shared(&mut self, shared: bool) -> Result<RecipeType, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("type")?;
        let name = self.expect_typename()?;
        self.expect_punct(&Token::LBrace)?;
        let mut fields = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let f = self.parse_field()?;
            fields.push(f);
            let _ = self.eat_punct(&Token::Semicolon) || self.eat_punct(&Token::Comma);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(RecipeType {
            name,
            fields,
            shared,
            span: self.span_to_here(start),
        })
    }

    fn parse_field(&mut self) -> Result<RecipeField, ParseError> {
        let name = self.expect_field_name()?;
        self.expect_punct(&Token::Colon)?;
        let ty = self.parse_field_type()?;
        let optional = self.eat_punct(&Token::Question);
        Ok(RecipeField { name, ty, optional })
    }

    /// Field-position identifiers can be keywords (e.g. `name: String`).
    fn expect_field_name(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Token::Ident(s)) | Some(Token::Keyword(s)) => {
                self.bump();
                Ok(s)
            }
            _ => Err(self.unexpected("field name")),
        }
    }

    /// Case-arm labels can be enum variants (TypeName / Ident) or scalar
    /// literals (true / false / null / int / string) when the scrutinee
    /// is a bool, int, string, or nullable value.
    fn expect_case_label(&mut self) -> Result<String, ParseError> {
        match self.peek().cloned() {
            Some(Token::Ident(s)) | Some(Token::TypeName(s)) | Some(Token::Keyword(s)) => {
                self.bump();
                Ok(s)
            }
            Some(Token::Bool(b)) => {
                self.bump();
                Ok(b.to_string())
            }
            Some(Token::Null) => {
                self.bump();
                Ok("null".into())
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(s)
            }
            Some(Token::Int(n)) => {
                self.bump();
                Ok(n.to_string())
            }
            _ => Err(self.unexpected("case label")),
        }
    }

    fn parse_field_type(&mut self) -> Result<FieldType, ParseError> {
        if self.eat_punct(&Token::LBracket) {
            let inner = self.parse_field_type()?;
            self.expect_punct(&Token::RBracket)?;
            return Ok(FieldType::Array(Box::new(inner)));
        }
        match self.peek().cloned() {
            Some(Token::Keyword(k)) if k == "String" => {
                self.bump();
                Ok(FieldType::String)
            }
            Some(Token::Keyword(k)) if k == "Int" => {
                self.bump();
                Ok(FieldType::Int)
            }
            Some(Token::Keyword(k)) if k == "Double" => {
                self.bump();
                Ok(FieldType::Double)
            }
            Some(Token::Keyword(k)) if k == "Bool" => {
                self.bump();
                Ok(FieldType::Bool)
            }
            Some(Token::Keyword(k)) if k == "Ref" => {
                self.bump();
                self.expect_punct(&Token::Lt)?;
                let target = self.expect_typename()?;
                self.expect_punct(&Token::Gt)?;
                Ok(FieldType::Ref(target))
            }
            Some(Token::TypeName(t)) => {
                self.bump();
                // Without resolution, treat any user-declared TypeName as a Record.
                // Validator distinguishes record vs enum by lookup.
                Ok(FieldType::Record(t))
            }
            _ => Err(self.unexpected("type")),
        }
    }

    /// enum_decl := 'share'? 'enum' TypeName '{' variant ... '}'
    fn parse_enum_decl_shared(&mut self, shared: bool) -> Result<RecipeEnum, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("enum")?;
        let name = self.expect_typename()?;
        self.expect_punct(&Token::LBrace)?;
        let mut variants = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let v = match self.peek().cloned() {
                Some(Token::Ident(s)) => s,
                Some(Token::TypeName(s)) => s,
                _ => return Err(self.unexpected("enum variant name")),
            };
            self.bump();
            variants.push(v);
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(RecipeEnum {
            name,
            variants,
            shared,
            span: self.span_to_here(start),
        })
    }

    fn parse_input_decl(&mut self) -> Result<InputDecl, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("input")?;
        let name = self.expect_field_name()?;
        self.expect_punct(&Token::Colon)?;
        let ty = self.parse_field_type()?;
        let optional = self.eat_punct(&Token::Question);
        Ok(InputDecl {
            name,
            ty,
            optional,
            span: self.span_to_here(start),
        })
    }

    // --- expressions -------------------------------------------------------

    /// Top-level expression. Pipes sit at the lowest precedence so the
    /// arithmetic ladder collapses into the pipe head; reading
    /// `$oz * 28 |> normalizeUnit` as `($oz * 28) |> normalizeUnit` is
    /// the right shape — pipes operate on whole computed values.
    ///
    /// Precedence (low → high):
    ///   pipe  `|`
    ///   add   `+`, `-`
    ///   mul   `*`, `/`, `%`
    ///   unary `-`
    ///   postfix `[expr]`
    ///   primary
    fn parse_extraction(&mut self) -> Result<ExtractionExpr, ParseError> {
        let head = self.parse_additive()?;
        if self.peek() == Some(&Token::Pipe) {
            let mut calls = Vec::new();
            while self.eat_punct(&Token::Pipe) {
                calls.push(self.parse_transform_call()?);
            }
            Ok(ExtractionExpr::Pipe(Box::new(head), calls))
        } else {
            Ok(head)
        }
    }

    fn parse_additive(&mut self) -> Result<ExtractionExpr, ParseError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Dash) => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_multiplicative()?;
            lhs = ExtractionExpr::BinaryOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<ExtractionExpr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                Some(Token::Percent) => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = ExtractionExpr::BinaryOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<ExtractionExpr, ParseError> {
        if self.eat_punct(&Token::Dash) {
            let operand = self.parse_unary()?;
            return Ok(ExtractionExpr::Unary {
                op: UnOp::Neg,
                operand: Box::new(operand),
            });
        }
        self.parse_postfix()
    }

    /// Bracket postfix on an arbitrary expression: `$captures[2]`,
    /// `$xs[$i + 1]`. The path-level `[N]` (literal integer on a
    /// `PathExpr`) is still handled in `parse_path`; that form is
    /// null-tolerant by design (scraping records routinely access
    /// `$x.range[0]` on possibly-empty arrays). The expression-level
    /// `Index` here is strict: out-of-bounds raises `IndexOutOfBounds`.
    fn parse_postfix(&mut self) -> Result<ExtractionExpr, ParseError> {
        let mut base = self.parse_primary_expr()?;
        // Bracket postfix only attaches to values that don't already
        // carry a path. A bare `$xs[0]` is a path expression with a
        // literal index baked in — handled inside `parse_path`. Once we
        // leave path territory (e.g. a `Call` or `StructLiteral` result),
        // `[expr]` is the only way to index into the value at runtime.
        while matches!(base, ExtractionExpr::Call { .. } | ExtractionExpr::StructLiteral { .. })
            && self.peek() == Some(&Token::LBracket)
        {
            self.bump();
            let index = self.parse_extraction()?;
            self.expect_punct(&Token::RBracket)?;
            base = ExtractionExpr::Index {
                base: Box::new(base),
                index: Box::new(index),
            };
        }
        Ok(base)
    }

    fn parse_transform_call(&mut self) -> Result<TransformCall, ParseError> {
        // Transform names are identifiers, but a few are also reserved
        // keywords elsewhere in the grammar (`match` in
        // `captures.match`, `name`/`value` as struct fields). Accept
        // those keywords here too — a transform name slot after `|`
        // is unambiguous.
        let name = match self.peek().cloned() {
            Some(Token::Ident(s)) => {
                self.bump();
                s
            }
            Some(Token::Keyword(s)) => {
                self.bump();
                s
            }
            _ => return Err(self.unexpected("transform name")),
        };
        let mut args = Vec::new();
        if self.eat_punct(&Token::LParen) && !self.eat_punct(&Token::RParen) {
            loop {
                args.push(self.parse_extraction()?);
                if !self.eat_punct(&Token::Comma) {
                    break;
                }
            }
            self.expect_punct(&Token::RParen)?;
        }
        Ok(TransformCall { name, args })
    }

    fn parse_primary_expr(&mut self) -> Result<ExtractionExpr, ParseError> {
        // case-of?
        if self.eat_keyword("case") {
            let scrutinee = self.parse_path()?;
            self.expect_keyword("of")?;
            self.expect_punct(&Token::LBrace)?;
            let mut branches = Vec::new();
            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let label = self.expect_case_label()?;
                self.expect_punct(&Token::CaseArrow)?;
                let arm = self.parse_extraction()?;
                branches.push((label, arm));
                let _ = self.eat_punct(&Token::Semicolon) || self.eat_punct(&Token::Comma);
            }
            self.expect_punct(&Token::RBrace)?;
            return Ok(ExtractionExpr::CaseOf {
                scrutinee,
                branches,
            });
        }

        // Struct literal `{ field: expr, ... }`. Distinguished from a
        // case-of by the leading `{` — struct literals never start with
        // a keyword.
        if self.peek() == Some(&Token::LBrace) {
            return self.parse_struct_literal();
        }

        // Regex literal `/pattern/flags` — compile at parse time so
        // malformed regexes surface here, not at runtime.
        if let Some(Token::RegexLit { pattern, flags }) = self.peek().cloned() {
            let span = self.current_span();
            self.bump();
            validate_regex_flags(&flags, &span)?;
            // Compile to verify the pattern; the AST keeps source text
            // only since `regex::Regex` doesn't implement `Serialize`.
            // The evaluator re-compiles (and caches per-recipe).
            if let Err(e) = build_regex(&pattern, &flags) {
                return Err(ParseError::InvalidRegex {
                    span,
                    message: e.to_string(),
                });
            }
            return Ok(ExtractionExpr::RegexLiteral(RegexLiteral {
                pattern,
                flags,
            }));
        }

        // Parenthesized sub-expression.
        if self.eat_punct(&Token::LParen) {
            let inner = self.parse_extraction()?;
            self.expect_punct(&Token::RParen)?;
            return Ok(inner);
        }

        // function call form `coalesce(a, b)` — Ident '(' …
        if let Some(Token::Ident(name)) = self.peek().cloned() {
            if matches!(self.peek_at(1), Some(Token::LParen)) {
                self.bump(); // ident
                self.bump(); // (
                let mut args = Vec::new();
                if !self.eat_punct(&Token::RParen) {
                    loop {
                        args.push(self.parse_extraction()?);
                        if !self.eat_punct(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect_punct(&Token::RParen)?;
                }
                return Ok(ExtractionExpr::Call { name, args });
            }
        }

        match self.peek().cloned() {
            // path forms ($, $input, $secret.X, $var, $.x ...)
            Some(Token::DollarRoot)
            | Some(Token::DollarInput)
            | Some(Token::DollarSecret)
            | Some(Token::DollarVar(_)) => {
                let p = self.parse_path()?;
                Ok(ExtractionExpr::Path(p))
            }
            // template / string
            Some(Token::Str(s)) => {
                self.bump();
                // For now treat string literals without interpolations as
                // `Literal(JSONValue::String(...))`. Interpolated templates
                // are produced where the parser explicitly calls
                // `parse_template_string` (e.g. step.url).
                if !s.contains('{') {
                    return Ok(ExtractionExpr::Literal(JSONValue::String(s)));
                }
                let t = compile_template(&s);
                Ok(ExtractionExpr::Template(t))
            }
            Some(Token::Int(n)) => {
                self.bump();
                Ok(ExtractionExpr::Literal(JSONValue::Int(n)))
            }
            Some(Token::Float(n)) => {
                self.bump();
                Ok(ExtractionExpr::Literal(JSONValue::Double(n)))
            }
            Some(Token::Bool(b)) => {
                self.bump();
                Ok(ExtractionExpr::Literal(JSONValue::Bool(b)))
            }
            Some(Token::Null) => {
                self.bump();
                Ok(ExtractionExpr::Literal(JSONValue::Null))
            }
            _ => Err(self.unexpected("expression")),
        }
    }

    /// Struct literal — `{ field: expr, field2: expr2 }`. Reuses the
    /// `FieldBinding` shape so duplicate-field detection and downstream
    /// validation share the path with `emit` bindings. Field bindings
    /// use `:` here (not `←`) to match JSON-like syntax authors expect
    /// for a literal object value; `←` belongs to `emit` blocks.
    fn parse_struct_literal(&mut self) -> Result<ExtractionExpr, ParseError> {
        self.expect_punct(&Token::LBrace)?;
        let mut fields: Vec<FieldBinding> = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let field_name = match self.peek().cloned() {
                Some(Token::Ident(s)) => s,
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("field name")),
            };
            self.bump();
            self.expect_punct(&Token::Colon)?;
            let expr = self.parse_extraction()?;
            fields.push(FieldBinding { field_name, expr });
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(ExtractionExpr::StructLiteral { fields })
    }

    /// path := head ('.' ident | '?.' ident | '[' Int ']' | '[*]')*
    fn parse_path(&mut self) -> Result<PathExpr, ParseError> {
        let mut head = match self.peek().cloned() {
            Some(Token::DollarRoot) => {
                self.bump();
                PathExpr::Current
            }
            Some(Token::DollarInput) => {
                self.bump();
                PathExpr::Input
            }
            Some(Token::DollarSecret) => {
                self.bump();
                self.expect_punct(&Token::Dot)?;
                let name = self.expect_ident()?;
                PathExpr::Secret(name)
            }
            Some(Token::DollarVar(s)) => {
                self.bump();
                PathExpr::Variable(s)
            }
            _ => return Err(self.unexpected("path expression head")),
        };
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.bump();
                    let field = match self.peek().cloned() {
                        Some(Token::Ident(s)) => s,
                        Some(Token::TypeName(s)) => s,
                        Some(Token::Keyword(s)) => s, // allow `.value`, `.name` etc.
                        _ => return Err(self.unexpected("field name")),
                    };
                    self.bump();
                    head = PathExpr::Field(Box::new(head), field);
                }
                Some(Token::QDot) => {
                    self.bump();
                    let field = match self.peek().cloned() {
                        Some(Token::Ident(s)) => s,
                        Some(Token::TypeName(s)) => s,
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("field name")),
                    };
                    self.bump();
                    head = PathExpr::OptField(Box::new(head), field);
                }
                Some(Token::LBracket) => {
                    self.bump();
                    let n = self.expect_int()?;
                    self.expect_punct(&Token::RBracket)?;
                    head = PathExpr::Index(Box::new(head), n);
                }
                Some(Token::Wildcard) => {
                    self.bump();
                    head = PathExpr::Wildcard(Box::new(head));
                }
                _ => break,
            }
        }
        Ok(head)
    }

    // --- statements --------------------------------------------------------

    /// statement := step | emit | for_loop
    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match self.peek().cloned() {
            Some(Token::Keyword(k)) if k == "step" => Ok(Statement::Step(self.parse_step()?)),
            Some(Token::Keyword(k)) if k == "emit" => Ok(Statement::Emit(self.parse_emit()?)),
            Some(Token::Keyword(k)) if k == "for" => self.parse_for_loop(),
            _ => Err(self.unexpected("statement (step | emit | for)")),
        }
    }

    fn parse_for_loop(&mut self) -> Result<Statement, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("for")?;
        let var = match self.peek().cloned() {
            Some(Token::DollarVar(s)) => {
                self.bump();
                s
            }
            _ => return Err(self.unexpected("loop variable ($name)")),
        };
        self.expect_keyword("in")?;
        let collection = self.parse_extraction()?;
        self.expect_punct(&Token::LBrace)?;
        let mut body = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            body.push(self.parse_statement()?);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(Statement::ForLoop {
            variable: var,
            collection,
            body,
            span: self.span_to_here(start),
        })
    }

    fn parse_emit(&mut self) -> Result<Emission, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("emit")?;
        let type_name = self.expect_typename()?;
        self.expect_punct(&Token::LBrace)?;
        let mut bindings = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let field_name = match self.peek().cloned() {
                Some(Token::Ident(s)) => s,
                Some(Token::Keyword(s)) => s, // allow keyword-shaped field names
                _ => return Err(self.unexpected("field name")),
            };
            self.bump();
            self.expect_punct(&Token::Arrow)?;
            let expr = self.parse_extraction()?;
            bindings.push(FieldBinding { field_name, expr });
            let _ = self.eat_punct(&Token::Semicolon) || self.eat_punct(&Token::Comma);
        }
        self.expect_punct(&Token::RBrace)?;
        // Optional post-fix `as $ident` — introduces a `Ref<TypeName>`
        // binding visible to subsequent statements in the enclosing
        // lexical scope.
        let bind_name = if matches!(self.peek(), Some(Token::Keyword(k)) if k == "as") {
            self.bump();
            match self.peek().cloned() {
                Some(Token::DollarVar(s)) => {
                    self.bump();
                    Some(s)
                }
                _ => return Err(self.unexpected("'$<name>'")),
            }
        } else {
            None
        };
        Ok(Emission {
            type_name,
            bindings,
            bind_name,
            span: self.span_to_here(start),
        })
    }

    fn parse_step(&mut self) -> Result<HTTPStep, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("step")?;
        let name = self.expect_field_name()?;
        self.expect_punct(&Token::LBrace)?;

        let mut method: Option<String> = None;
        let mut url: Option<Template> = None;
        let mut headers: Vec<(String, Template)> = Vec::new();
        let mut body: Option<HTTPBody> = None;
        let mut pagination: Option<Pagination> = None;
        let mut extract: Option<RegexExtract> = None;

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            match self.peek().cloned() {
                Some(Token::Keyword(k)) if k == "method" => {
                    self.bump();
                    let s = self.expect_string()?;
                    method = Some(s);
                }
                Some(Token::Keyword(k)) if k == "url" => {
                    self.bump();
                    let s = self.expect_string()?;
                    url = Some(compile_template(&s));
                }
                Some(Token::Keyword(k)) if k == "headers" => {
                    self.bump();
                    self.expect_punct(&Token::LBrace)?;
                    while !matches!(self.peek(), Some(Token::RBrace) | None) {
                        let key = self.expect_string()?;
                        self.expect_punct(&Token::Colon)?;
                        let val = self.expect_string()?;
                        headers.push((key, compile_template(&val)));
                        let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                    }
                    self.expect_punct(&Token::RBrace)?;
                }
                Some(Token::Keyword(k)) if k == "body" => {
                    self.bump();
                    self.expect_punct(&Token::Dot)?;
                    let kind = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("'json' | 'form' | 'raw'")),
                    };
                    self.bump();
                    self.expect_punct(&Token::LBrace)?;
                    body = Some(match kind.as_str() {
                        "json" => HTTPBody::JsonObject(self.parse_json_body_kvs()?),
                        "form" => HTTPBody::Form(self.parse_form_body_kvs()?),
                        "raw" => {
                            let s = self.expect_string()?;
                            HTTPBody::Raw(compile_template(&s))
                        }
                        other => {
                            return Err(self.generic(&format!(
                                "unknown body kind '{other}' (expected json|form|raw)"
                            )));
                        }
                    });
                    self.expect_punct(&Token::RBrace)?;
                }
                Some(Token::Keyword(k)) if k == "paginate" => {
                    pagination = Some(self.parse_pagination()?);
                }
                Some(Token::Keyword(k)) if k == "extract" => {
                    self.bump();
                    self.expect_punct(&Token::Dot)?;
                    let kind = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("'regex'")),
                    };
                    self.bump();
                    if kind != "regex" {
                        return Err(self.generic(&format!(
                            "unknown extract kind '{kind}' (expected 'regex')"
                        )));
                    }
                    self.expect_punct(&Token::LBrace)?;
                    let mut pattern: Option<String> = None;
                    let mut groups: Vec<String> = Vec::new();
                    while !matches!(self.peek(), Some(Token::RBrace) | None) {
                        let k = match self.peek().cloned() {
                            Some(Token::Keyword(s)) | Some(Token::Ident(s)) => s,
                            _ => return Err(self.unexpected("extract field")),
                        };
                        self.bump();
                        self.expect_punct(&Token::Colon)?;
                        match k.as_str() {
                            "pattern" => pattern = Some(self.expect_string()?),
                            "groups" => {
                                self.expect_punct(&Token::LBracket)?;
                                while !matches!(self.peek(), Some(Token::RBracket) | None) {
                                    let g = match self.peek().cloned() {
                                        Some(Token::Ident(s)) | Some(Token::Keyword(s)) => s,
                                        Some(Token::Str(s)) => s,
                                        _ => return Err(self.unexpected("group name")),
                                    };
                                    self.bump();
                                    groups.push(g);
                                    let _ = self.eat_punct(&Token::Comma);
                                }
                                self.expect_punct(&Token::RBracket)?;
                            }
                            other => {
                                return Err(
                                    self.generic(&format!("unknown extract.regex field '{other}'"))
                                );
                            }
                        }
                        let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                    }
                    self.expect_punct(&Token::RBrace)?;
                    extract = Some(RegexExtract {
                        pattern: pattern
                            .ok_or_else(|| self.generic("extract.regex missing 'pattern'"))?,
                        groups,
                    });
                }
                Some(other) => {
                    return Err(self.generic(&format!("unknown step field: {}", other.describe())));
                }
                None => break,
            }
        }
        self.expect_punct(&Token::RBrace)?;

        let req = HTTPRequest {
            method: method.ok_or_else(|| self.generic("step missing 'method'"))?,
            url: url.ok_or_else(|| self.generic("step missing 'url'"))?,
            headers,
            body,
        };
        Ok(HTTPStep {
            name,
            request: req,
            pagination,
            extract,
            span: self.span_to_here(start),
        })
    }

    fn parse_json_body_kvs(&mut self) -> Result<Vec<HTTPBodyKV>, ParseError> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Ident(s)) => s,
                Some(Token::Keyword(s)) => s,
                Some(Token::Str(s)) => s,
                _ => return Err(self.unexpected("body field key")),
            };
            self.bump();
            self.expect_punct(&Token::Colon)?;
            let value = self.parse_body_value()?;
            out.push(HTTPBodyKV { key, value });
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        Ok(out)
    }

    fn parse_form_body_kvs(&mut self) -> Result<Vec<(String, BodyValue)>, ParseError> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = self.expect_string()?;
            self.expect_punct(&Token::Colon)?;
            let value = self.parse_body_value()?;
            out.push((key, value));
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        Ok(out)
    }

    fn parse_body_value(&mut self) -> Result<BodyValue, ParseError> {
        // Nested object?
        if self.peek() == Some(&Token::LBrace) {
            self.bump();
            let kvs = self.parse_json_body_kvs()?;
            self.expect_punct(&Token::RBrace)?;
            return Ok(BodyValue::Object(kvs));
        }
        // Array?
        if self.peek() == Some(&Token::LBracket) {
            self.bump();
            let mut items = Vec::new();
            if !self.eat_punct(&Token::RBracket) {
                loop {
                    items.push(self.parse_body_value()?);
                    if !self.eat_punct(&Token::Comma) {
                        break;
                    }
                }
                self.expect_punct(&Token::RBracket)?;
            }
            return Ok(BodyValue::Array(items));
        }
        // case-of?
        if matches!(self.peek(), Some(Token::Keyword(k)) if k == "case") {
            self.bump();
            let scrutinee = self.parse_path()?;
            self.expect_keyword("of")?;
            self.expect_punct(&Token::LBrace)?;
            let mut branches = Vec::new();
            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let label = self.expect_case_label()?;
                self.expect_punct(&Token::CaseArrow)?;
                let val = self.parse_body_value()?;
                branches.push((label, val));
                let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
            }
            self.expect_punct(&Token::RBrace)?;
            return Ok(BodyValue::CaseOf {
                scrutinee,
                branches,
            });
        }
        // Path?
        match self.peek() {
            Some(Token::DollarRoot)
            | Some(Token::DollarInput)
            | Some(Token::DollarSecret)
            | Some(Token::DollarVar(_)) => {
                let p = self.parse_path()?;
                return Ok(BodyValue::Path(p));
            }
            _ => {}
        }
        // String / number / bool / null literals.
        match self.peek().cloned() {
            Some(Token::Str(s)) => {
                self.bump();
                if s.contains('{') {
                    Ok(BodyValue::TemplateString(compile_template(&s)))
                } else {
                    Ok(BodyValue::Literal(JSONValue::String(s)))
                }
            }
            Some(Token::Int(n)) => {
                self.bump();
                Ok(BodyValue::Literal(JSONValue::Int(n)))
            }
            Some(Token::Float(n)) => {
                self.bump();
                Ok(BodyValue::Literal(JSONValue::Double(n)))
            }
            Some(Token::Bool(b)) => {
                self.bump();
                Ok(BodyValue::Literal(JSONValue::Bool(b)))
            }
            Some(Token::Null) => {
                self.bump();
                Ok(BodyValue::Literal(JSONValue::Null))
            }
            _ => Err(self.unexpected("body value")),
        }
    }

    fn parse_pagination(&mut self) -> Result<Pagination, ParseError> {
        self.expect_keyword("paginate")?;
        let kind = match self.peek().cloned() {
            Some(Token::Keyword(s)) => s,
            _ => return Err(self.unexpected("pagination strategy")),
        };
        self.bump();
        self.expect_punct(&Token::LBrace)?;
        let mut items_path: Option<PathExpr> = None;
        let mut total_path: Option<PathExpr> = None;
        let mut cursor_path: Option<PathExpr> = None;
        let mut page_param: Option<String> = None;
        let mut cursor_param: Option<String> = None;
        let mut page_size: u32 = 0;
        let mut page_zero_indexed = false;

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("pagination field")),
            };
            self.bump();
            self.expect_punct(&Token::Colon)?;
            match key.as_str() {
                "items" => items_path = Some(self.parse_path()?),
                "total" => total_path = Some(self.parse_path()?),
                "cursorPath" => cursor_path = Some(self.parse_path()?),
                "pageParam" => page_param = Some(self.expect_string()?),
                "cursorParam" => cursor_param = Some(self.expect_string()?),
                "pageSize" => page_size = self.expect_int()? as u32,
                "pageZeroIndexed" => {
                    page_zero_indexed = match self.peek().cloned() {
                        Some(Token::Bool(b)) => {
                            self.bump();
                            b
                        }
                        _ => return Err(self.unexpected("boolean")),
                    }
                }
                other => {
                    return Err(self.generic(&format!("unknown paginate field '{other}'")));
                }
            }
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;

        let pag = match kind.as_str() {
            "pageWithTotal" => Pagination::PageWithTotal {
                items_path: items_path
                    .ok_or_else(|| self.generic("pageWithTotal missing 'items'"))?,
                total_path: total_path
                    .ok_or_else(|| self.generic("pageWithTotal missing 'total'"))?,
                page_param: page_param
                    .ok_or_else(|| self.generic("pageWithTotal missing 'pageParam'"))?,
                page_size,
                page_zero_indexed,
            },
            "untilEmpty" => Pagination::UntilEmpty {
                items_path: items_path.ok_or_else(|| self.generic("untilEmpty missing 'items'"))?,
                page_param: page_param
                    .ok_or_else(|| self.generic("untilEmpty missing 'pageParam'"))?,
                page_zero_indexed,
            },
            "cursor" => Pagination::Cursor {
                items_path: items_path.ok_or_else(|| self.generic("cursor missing 'items'"))?,
                cursor_path: cursor_path
                    .ok_or_else(|| self.generic("cursor missing 'cursorPath'"))?,
                cursor_param: cursor_param
                    .ok_or_else(|| self.generic("cursor missing 'cursorParam'"))?,
            },
            other => return Err(self.generic(&format!("unknown pagination strategy '{other}'"))),
        };
        Ok(pag)
    }

    // --- auth --------------------------------------------------------------

    fn parse_auth(&mut self) -> Result<AuthStrategy, ParseError> {
        self.expect_keyword("auth")?;
        self.expect_punct(&Token::Dot)?;
        let kind = match self.peek().cloned() {
            Some(Token::Keyword(s)) => s,
            _ => return Err(self.unexpected("auth strategy")),
        };
        self.bump();

        match kind.as_str() {
            "staticHeader" => {
                self.expect_punct(&Token::LBrace)?;
                let mut name: Option<String> = None;
                let mut value: Option<Template> = None;
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    let k = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("field name")),
                    };
                    self.bump();
                    self.expect_punct(&Token::Colon)?;
                    match k.as_str() {
                        "name" => name = Some(self.expect_string()?),
                        "value" => value = Some(compile_template(&self.expect_string()?)),
                        other => {
                            return Err(
                                self.generic(&format!("unknown staticHeader field '{other}'"))
                            );
                        }
                    }
                    let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                }
                self.expect_punct(&Token::RBrace)?;
                Ok(AuthStrategy::StaticHeader {
                    name: name.ok_or_else(|| self.generic("staticHeader missing 'name'"))?,
                    value: value.ok_or_else(|| self.generic("staticHeader missing 'value'"))?,
                })
            }
            "htmlPrime" => {
                self.expect_punct(&Token::LBrace)?;
                let mut step_name: Option<String> = None;
                let mut captured_vars: Vec<HtmlPrimeVar> = Vec::new();
                // Simplified: accept `step: <ident>` then a list of regex vars
                // captured via a sibling regex.extract block inside the step.
                // For initial parity we collect just `stepName`, `nonceVar`,
                // `ajaxUrlVar` plain idents; full regex group binding is in R1.4+.
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    let k = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        Some(Token::Ident(s)) => s,
                        _ => return Err(self.unexpected("field name")),
                    };
                    self.bump();
                    self.expect_punct(&Token::Colon)?;
                    match k.as_str() {
                        "step" | "stepName" => {
                            let s = match self.peek().cloned() {
                                Some(Token::Ident(s)) => s,
                                Some(Token::Str(s)) => s,
                                _ => return Err(self.unexpected("step name")),
                            };
                            self.bump();
                            step_name = Some(s);
                        }
                        "nonceVar" => {
                            let s = self.expect_string()?;
                            captured_vars.push(HtmlPrimeVar {
                                var_name: s,
                                regex_pattern: String::new(),
                                group_index: 0,
                            });
                        }
                        "ajaxUrlVar" => {
                            let s = self.expect_string()?;
                            captured_vars.push(HtmlPrimeVar {
                                var_name: s,
                                regex_pattern: String::new(),
                                group_index: 0,
                            });
                        }
                        other => {
                            return Err(self.generic(&format!("unknown htmlPrime field '{other}'")));
                        }
                    }
                    let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                }
                self.expect_punct(&Token::RBrace)?;
                Ok(AuthStrategy::HtmlPrime {
                    step_name: step_name.ok_or_else(|| self.generic("htmlPrime missing 'step'"))?,
                    captured_vars,
                })
            }
            "session" => {
                self.expect_punct(&Token::Dot)?;
                let variant = match self.peek().cloned() {
                    Some(Token::Keyword(s)) => s,
                    _ => return Err(self.unexpected("session variant")),
                };
                self.bump();
                self.parse_session_auth(&variant)
            }
            other => Err(self.generic(&format!("auth strategy '{other}' not yet supported"))),
        }
    }

    fn parse_session_auth(&mut self, variant: &str) -> Result<AuthStrategy, ParseError> {
        self.expect_punct(&Token::LBrace)?;
        let mut url: Option<Template> = None;
        let mut method: Option<String> = None;
        let mut body: Option<HTTPBody> = None;
        let mut token_path: Option<PathExpr> = None;
        let mut header_name: String = "Authorization".into();
        let mut header_prefix: String = "Bearer ".into();
        let mut source_path: Option<Template> = None;
        let mut format = CookieFormat::Json;
        let mut capture_cookies: bool = true;
        let mut max_reauth_retries: u32 = 1;
        let mut cache_duration_secs: Option<u64> = None;
        let mut cache_encrypted = false;
        let mut requires_mfa = false;
        let mut mfa_field_name: String = "code".into();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("session field")),
            };
            self.bump();
            match key.as_str() {
                "url" => {
                    self.expect_punct(&Token::Colon)?;
                    url = Some(compile_template(&self.expect_string()?));
                }
                "method" => {
                    self.expect_punct(&Token::Colon)?;
                    method = Some(self.expect_string()?);
                }
                "body" => {
                    self.expect_punct(&Token::Dot)?;
                    let kind = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("'json' | 'form' | 'raw'")),
                    };
                    self.bump();
                    self.expect_punct(&Token::LBrace)?;
                    body = Some(match kind.as_str() {
                        "json" => HTTPBody::JsonObject(self.parse_json_body_kvs()?),
                        "form" => HTTPBody::Form(self.parse_form_body_kvs()?),
                        "raw" => HTTPBody::Raw(compile_template(&self.expect_string()?)),
                        other => {
                            return Err(self.generic(&format!("unknown body kind '{other}'")));
                        }
                    });
                    self.expect_punct(&Token::RBrace)?;
                }
                "tokenPath" => {
                    self.expect_punct(&Token::Colon)?;
                    token_path = Some(self.parse_path()?);
                }
                "headerName" => {
                    self.expect_punct(&Token::Colon)?;
                    header_name = self.expect_string()?;
                }
                "headerPrefix" => {
                    self.expect_punct(&Token::Colon)?;
                    header_prefix = self.expect_string()?;
                }
                "sourcePath" => {
                    self.expect_punct(&Token::Colon)?;
                    source_path = Some(compile_template(&self.expect_string()?));
                }
                "format" => {
                    self.expect_punct(&Token::Colon)?;
                    let s = match self.peek().cloned() {
                        Some(Token::Str(s)) => {
                            self.bump();
                            s
                        }
                        Some(Token::Ident(s)) | Some(Token::Keyword(s)) => {
                            self.bump();
                            s
                        }
                        _ => return Err(self.unexpected("'json' or 'netscape'")),
                    };
                    format = match s.as_str() {
                        "json" => CookieFormat::Json,
                        "netscape" => CookieFormat::Netscape,
                        other => {
                            return Err(self.generic(&format!("unknown cookie format '{other}'")));
                        }
                    };
                }
                "captureCookies" => {
                    self.expect_punct(&Token::Colon)?;
                    capture_cookies = match self.peek().cloned() {
                        Some(Token::Bool(b)) => {
                            self.bump();
                            b
                        }
                        _ => return Err(self.unexpected("boolean")),
                    };
                }
                "maxReauthRetries" => {
                    self.expect_punct(&Token::Colon)?;
                    max_reauth_retries = self.expect_int()? as u32;
                }
                "cache" => {
                    self.expect_punct(&Token::Colon)?;
                    cache_duration_secs = Some(self.expect_int()? as u64);
                }
                "cacheEncrypted" => {
                    self.expect_punct(&Token::Colon)?;
                    cache_encrypted = match self.peek().cloned() {
                        Some(Token::Bool(b)) => {
                            self.bump();
                            b
                        }
                        _ => return Err(self.unexpected("boolean")),
                    };
                }
                "requiresMFA" => {
                    self.expect_punct(&Token::Colon)?;
                    requires_mfa = match self.peek().cloned() {
                        Some(Token::Bool(b)) => {
                            self.bump();
                            b
                        }
                        _ => return Err(self.unexpected("boolean")),
                    };
                }
                "mfaFieldName" => {
                    self.expect_punct(&Token::Colon)?;
                    mfa_field_name = self.expect_string()?;
                }
                other => {
                    return Err(self.generic(&format!("unknown session field '{other}'")));
                }
            }
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;

        let kind = match variant {
            "formLogin" => SessionKind::FormLogin(FormLogin {
                url: url.ok_or_else(|| self.generic("formLogin missing 'url'"))?,
                method: method.unwrap_or_else(|| "POST".into()),
                body: body.unwrap_or(HTTPBody::JsonObject(vec![])),
                capture_cookies,
            }),
            "bearerLogin" => SessionKind::BearerLogin(BearerLogin {
                url: url.ok_or_else(|| self.generic("bearerLogin missing 'url'"))?,
                method: method.unwrap_or_else(|| "POST".into()),
                body: body.unwrap_or(HTTPBody::JsonObject(vec![])),
                token_path: token_path
                    .ok_or_else(|| self.generic("bearerLogin missing 'tokenPath'"))?,
                header_name,
                header_prefix,
            }),
            "cookiePersist" => SessionKind::CookiePersist(CookiePersist {
                source_path: source_path
                    .ok_or_else(|| self.generic("cookiePersist missing 'sourcePath'"))?,
                format,
            }),
            other => return Err(self.generic(&format!("unknown session variant '{other}'"))),
        };

        Ok(AuthStrategy::Session(SessionAuth {
            kind,
            max_reauth_retries,
            cache_duration_secs,
            cache_encrypted,
            requires_mfa,
            mfa_field_name,
        }))
    }

    // --- browser block -----------------------------------------------------

    fn parse_browser_block(&mut self) -> Result<BrowserConfig, ParseError> {
        self.expect_keyword("browser")?;
        self.expect_punct(&Token::LBrace)?;

        let mut initial_url: Option<Template> = None;
        let mut age_gate: Option<AgeGateConfig> = None;
        let mut dismissals: Option<DismissalConfig> = None;
        let mut warmup_clicks: Vec<String> = Vec::new();
        let mut observe: Option<String> = None;
        let mut pagination: Option<BrowserPaginationConfig> = None;
        let mut captures: Vec<CaptureRule> = Vec::new();
        let mut document_capture: Option<DocumentCaptureRule> = None;
        let mut interactive: Option<InteractiveConfig> = None;

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("browser field")),
            };
            match key.as_str() {
                "initialURL" => {
                    self.bump();
                    self.expect_punct(&Token::Colon)?;
                    let s = self.expect_string()?;
                    initial_url = Some(compile_template(&s));
                }
                "observe" => {
                    self.bump();
                    self.expect_punct(&Token::Colon)?;
                    observe = Some(self.expect_string()?);
                }
                "ageGate" => {
                    self.bump();
                    self.expect_punct(&Token::Dot)?;
                    self.expect_keyword("autoFill")?;
                    self.expect_punct(&Token::LBrace)?;
                    let mut y = 0;
                    let mut m = 0;
                    let mut d = 0;
                    let mut reload_after = true;
                    while !matches!(self.peek(), Some(Token::RBrace) | None) {
                        let k = match self.peek().cloned() {
                            Some(Token::Keyword(s)) => s,
                            _ => return Err(self.unexpected("ageGate field")),
                        };
                        self.bump();
                        self.expect_punct(&Token::Colon)?;
                        match k.as_str() {
                            "dob" => match self.peek().cloned() {
                                Some(Token::Date { year, month, day }) => {
                                    self.bump();
                                    y = year as u32;
                                    m = month;
                                    d = day;
                                }
                                _ => return Err(self.unexpected("date literal YYYY-MM-DD")),
                            },
                            "reloadAfter" | "reloadAfterSubmit" => {
                                reload_after = match self.peek().cloned() {
                                    Some(Token::Bool(b)) => {
                                        self.bump();
                                        b
                                    }
                                    _ => return Err(self.unexpected("boolean")),
                                };
                            }
                            other => {
                                return Err(
                                    self.generic(&format!("unknown ageGate field '{other}'"))
                                );
                            }
                        }
                        let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                    }
                    self.expect_punct(&Token::RBrace)?;
                    age_gate = Some(AgeGateConfig {
                        year: y,
                        month: m,
                        day: d,
                        reload_after,
                    });
                }
                "dismissals" => {
                    self.bump();
                    self.expect_punct(&Token::LBrace)?;
                    let mut max_attempts: u32 = 8;
                    let mut extra_labels = Vec::new();
                    while !matches!(self.peek(), Some(Token::RBrace) | None) {
                        let k = match self.peek().cloned() {
                            Some(Token::Keyword(s)) => s,
                            _ => return Err(self.unexpected("dismissals field")),
                        };
                        self.bump();
                        self.expect_punct(&Token::Colon)?;
                        match k.as_str() {
                            "maxIterations" => max_attempts = self.expect_int()? as u32,
                            "extraLabels" => {
                                self.expect_punct(&Token::LBracket)?;
                                while !matches!(self.peek(), Some(Token::RBracket) | None) {
                                    let s = self.expect_string()?;
                                    extra_labels.push(s);
                                    let _ = self.eat_punct(&Token::Comma);
                                }
                                self.expect_punct(&Token::RBracket)?;
                            }
                            other => {
                                return Err(
                                    self.generic(&format!("unknown dismissals field '{other}'"))
                                );
                            }
                        }
                        let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
                    }
                    self.expect_punct(&Token::RBrace)?;
                    dismissals = Some(DismissalConfig {
                        max_attempts,
                        extra_labels,
                    });
                }
                "warmupClicks" => {
                    self.bump();
                    self.expect_punct(&Token::Colon)?;
                    self.expect_punct(&Token::LBracket)?;
                    while !matches!(self.peek(), Some(Token::RBracket) | None) {
                        let s = self.expect_string()?;
                        warmup_clicks.push(s);
                        let _ = self.eat_punct(&Token::Comma);
                    }
                    self.expect_punct(&Token::RBracket)?;
                }
                "paginate" => {
                    pagination = Some(self.parse_browser_pagination()?);
                }
                "captures" => {
                    self.bump();
                    self.expect_punct(&Token::Dot)?;
                    let kind = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        _ => return Err(self.unexpected("'match' or 'document'")),
                    };
                    self.bump();
                    self.expect_punct(&Token::LBrace)?;
                    match kind.as_str() {
                        "match" => {
                            let r = self.parse_capture_match_body()?;
                            captures.push(r);
                        }
                        "document" => {
                            if document_capture.is_some() {
                                return Err(self.generic("duplicate captures.document"));
                            }
                            let r = self.parse_capture_document_body()?;
                            document_capture = Some(r);
                        }
                        other => {
                            return Err(self.generic(&format!("unknown captures.{other}")));
                        }
                    }
                    self.expect_punct(&Token::RBrace)?;
                }
                "interactive" => {
                    if interactive.is_some() {
                        return Err(self.generic("duplicate interactive block"));
                    }
                    interactive = Some(self.parse_interactive()?);
                }
                other => {
                    return Err(self.generic(&format!("unknown browser field '{other}'")));
                }
            }
        }
        self.expect_punct(&Token::RBrace)?;

        Ok(BrowserConfig {
            initial_url: initial_url.ok_or_else(|| self.generic("browser missing 'initialURL'"))?,
            age_gate,
            dismissals,
            warmup_clicks,
            observe: observe.ok_or_else(|| self.generic("browser missing 'observe'"))?,
            pagination: pagination.ok_or_else(|| self.generic("browser missing 'paginate'"))?,
            captures,
            document_capture,
            interactive,
        })
    }

    fn parse_browser_pagination(&mut self) -> Result<BrowserPaginationConfig, ParseError> {
        // Forms accepted:
        //   `paginate browserPaginate.scroll { … }`  — canonical
        //   `paginate.scroll { … }`                  — shorthand
        self.expect_keyword("paginate")?;
        let saw_dot = self.eat_punct(&Token::Dot);
        let mut mode_kw = match self.peek().cloned() {
            Some(Token::Keyword(s)) => s,
            _ => return Err(self.unexpected("'scroll' or 'replay'")),
        };
        self.bump();
        // If we saw `paginate browserPaginate`, dig in one more level.
        if mode_kw == "browserPaginate" && !saw_dot {
            self.expect_punct(&Token::Dot)?;
            mode_kw = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("'scroll' or 'replay'")),
            };
            self.bump();
        }
        let mode = match mode_kw.as_str() {
            "scroll" => BrowserPaginationMode::Scroll,
            "replay" => BrowserPaginationMode::Replay,
            other => {
                return Err(self.generic(&format!("unknown browser paginate '{other}'")));
            }
        };
        self.expect_punct(&Token::LBrace)?;
        let mut until: Option<BrowserPaginateUntil> = None;
        let mut max_iterations: u32 = 30;
        let mut iteration_delay_secs: f64 = 1.8;
        let mut seed_filter: Option<String> = None;
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("paginate field")),
            };
            self.bump();
            self.expect_punct(&Token::Colon)?;
            match key.as_str() {
                "until" => {
                    let fn_name = match self.peek().cloned() {
                        Some(Token::Keyword(s)) => s,
                        Some(Token::Ident(s)) => s,
                        _ => return Err(self.unexpected("until predicate")),
                    };
                    self.bump();
                    self.expect_punct(&Token::LParen)?;
                    match fn_name.as_str() {
                        "noProgressFor" => {
                            let n = self.expect_int()? as u32;
                            until = Some(BrowserPaginateUntil::NoProgressFor(n));
                        }
                        other => {
                            return Err(self.generic(&format!("unknown until predicate '{other}'")));
                        }
                    }
                    self.expect_punct(&Token::RParen)?;
                }
                "maxIterations" => max_iterations = self.expect_int()? as u32,
                "iterationDelay" => match self.peek().cloned() {
                    Some(Token::Float(f)) => {
                        self.bump();
                        iteration_delay_secs = f;
                    }
                    Some(Token::Int(n)) => {
                        self.bump();
                        iteration_delay_secs = n as f64;
                    }
                    _ => return Err(self.unexpected("number")),
                },
                "seedFilter" => seed_filter = Some(self.expect_string()?),
                other => {
                    return Err(self.generic(&format!("unknown paginate field '{other}'")));
                }
            }
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(BrowserPaginationConfig {
            mode,
            until: until.ok_or_else(|| self.generic("paginate missing 'until'"))?,
            max_iterations,
            iteration_delay_secs,
            seed_filter,
            replay_override: Vec::new(),
        })
    }

    fn parse_capture_match_body(&mut self) -> Result<CaptureRule, ParseError> {
        // First field is `urlPattern: "..."`, then a `for $x in <expr> { … }`.
        self.expect_keyword("urlPattern")?;
        self.expect_punct(&Token::Colon)?;
        let url_pattern = self.expect_string()?;

        self.expect_keyword("for")?;
        let iter_var = match self.peek().cloned() {
            Some(Token::DollarVar(s)) => {
                self.bump();
                s
            }
            _ => return Err(self.unexpected("loop variable ($name)")),
        };
        self.expect_keyword("in")?;
        let iter_path = self.parse_extraction()?;
        self.expect_punct(&Token::LBrace)?;
        let mut body = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            body.push(self.parse_statement()?);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(CaptureRule {
            url_pattern,
            iter_var,
            iter_path,
            body,
        })
    }

    fn parse_capture_document_body(&mut self) -> Result<DocumentCaptureRule, ParseError> {
        self.expect_keyword("for")?;
        let iter_var = match self.peek().cloned() {
            Some(Token::DollarVar(s)) => {
                self.bump();
                s
            }
            _ => return Err(self.unexpected("loop variable ($name)")),
        };
        self.expect_keyword("in")?;
        let iter_path = self.parse_extraction()?;
        self.expect_punct(&Token::LBrace)?;
        let mut body = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            body.push(self.parse_statement()?);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(DocumentCaptureRule {
            iter_var,
            iter_path,
            body,
        })
    }

    fn parse_interactive(&mut self) -> Result<InteractiveConfig, ParseError> {
        self.expect_keyword("interactive")?;
        self.expect_punct(&Token::LBrace)?;
        let mut bootstrap_url: Option<Template> = None;
        let mut cookie_domains: Vec<String> = Vec::new();
        let mut session_expired_pattern: Option<String> = None;
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let key = match self.peek().cloned() {
                Some(Token::Keyword(s)) => s,
                _ => return Err(self.unexpected("interactive field")),
            };
            self.bump();
            self.expect_punct(&Token::Colon)?;
            match key.as_str() {
                "bootstrapURL" => {
                    let s = self.expect_string()?;
                    bootstrap_url = Some(compile_template(&s));
                }
                "cookieDomains" => {
                    self.expect_punct(&Token::LBracket)?;
                    while !matches!(self.peek(), Some(Token::RBracket) | None) {
                        cookie_domains.push(self.expect_string()?);
                        let _ = self.eat_punct(&Token::Comma);
                    }
                    self.expect_punct(&Token::RBracket)?;
                }
                "sessionExpiredPattern" => {
                    session_expired_pattern = Some(self.expect_string()?);
                }
                other => {
                    return Err(self.generic(&format!("unknown interactive field '{other}'")));
                }
            }
            let _ = self.eat_punct(&Token::Comma) || self.eat_punct(&Token::Semicolon);
        }
        self.expect_punct(&Token::RBrace)?;
        Ok(InteractiveConfig {
            bootstrap_url,
            cookie_domains,
            session_expired_pattern,
        })
    }

    // --- expect ------------------------------------------------------------

    fn parse_expect_block(&mut self) -> Result<Expectation, ParseError> {
        let start = self.current_span().start;
        self.expect_keyword("expect")?;
        self.expect_punct(&Token::LBrace)?;
        // `records.where(typeName == "X").count <op> N`
        self.expect_keyword("records")?;
        self.expect_punct(&Token::Dot)?;
        self.expect_keyword("where")?;
        self.expect_punct(&Token::LParen)?;
        self.expect_keyword("typeName")?;
        self.expect_punct(&Token::Equal)?;
        self.expect_punct(&Token::Equal)?;
        let type_name = self.expect_string()?;
        self.expect_punct(&Token::RParen)?;
        self.expect_punct(&Token::Dot)?;
        self.expect_keyword("count")?;
        let op = self.parse_cmp_op()?;
        let value = self.expect_int()?;
        self.expect_punct(&Token::RBrace)?;
        Ok(Expectation {
            kind: ExpectationKind::RecordCount {
                type_name,
                op,
                value,
            },
            span: self.span_to_here(start),
        })
    }

    fn parse_cmp_op(&mut self) -> Result<ComparisonOp, ParseError> {
        match self.peek().cloned() {
            Some(Token::Gt) => {
                self.bump();
                if self.eat_punct(&Token::Equal) {
                    Ok(ComparisonOp::Ge)
                } else {
                    Ok(ComparisonOp::Gt)
                }
            }
            Some(Token::Lt) => {
                self.bump();
                if self.eat_punct(&Token::Equal) {
                    Ok(ComparisonOp::Le)
                } else {
                    Ok(ComparisonOp::Lt)
                }
            }
            Some(Token::Equal) => {
                self.bump();
                self.expect_punct(&Token::Equal)?;
                Ok(ComparisonOp::Eq)
            }
            Some(Token::Bang) => {
                self.bump();
                self.expect_punct(&Token::Equal)?;
                Ok(ComparisonOp::Ne)
            }
            _ => Err(self.unexpected("comparison operator")),
        }
    }

    fn generic(&self, msg: &str) -> ParseError {
        ParseError::Generic {
            span: self.current_span(),
            message: msg.into(),
        }
    }
}

/// Compile a raw template string with `{expr}` interpolations into a
/// `Template`. For each `{...}` segment, re-lex+parse the inner text as an
/// `ExtractionExpr`; literal text becomes `TemplatePart::Literal`.
pub fn compile_template(raw: &str) -> Template {
    let mut parts = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1).copied() != Some('{') {
            if !buf.is_empty() {
                parts.push(TemplatePart::Literal(std::mem::take(&mut buf)));
            }
            let mut depth = 1;
            let mut inner = String::new();
            i += 1;
            while i < chars.len() && depth > 0 {
                let c = chars[i];
                if c == '{' {
                    depth += 1;
                    inner.push(c);
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    inner.push(c);
                } else {
                    inner.push(c);
                }
                i += 1;
            }
            // Consume the closing brace.
            i += 1;
            // Re-parse the inner as an extraction expression.
            let toks = match lex(&inner) {
                Ok(t) => t,
                Err(_) => {
                    // Fall back to literal on lex failure.
                    parts.push(TemplatePart::Literal(format!("{{{inner}}}")));
                    continue;
                }
            };
            let mut p = Parser::new(toks, inner.len());
            match p.parse_extraction() {
                Ok(expr) => parts.push(TemplatePart::Interp(expr)),
                Err(_) => parts.push(TemplatePart::Literal(format!("{{{inner}}}"))),
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        parts.push(TemplatePart::Literal(buf));
    }
    Template { parts }
}

/// Allowed regex flags: `i` (case-insensitive), `m` (multi-line), `s`
/// (dot matches newline), `u` (Unicode-aware). JS-style `g` (global)
/// and `y` (sticky) aren't supported — the match/matches/replaceAll
/// transforms apply the regex once per call, so a global flag has no
/// runtime meaning.
fn validate_regex_flags(flags: &str, span: &std::ops::Range<usize>) -> Result<(), ParseError> {
    for ch in flags.chars() {
        if !matches!(ch, 'i' | 'm' | 's' | 'u') {
            return Err(ParseError::InvalidRegexFlag {
                span: span.clone(),
                flag: ch,
            });
        }
    }
    Ok(())
}

/// Build a `regex::Regex` from pattern + flags. Exposed at module-level
/// because the evaluator wants the same compilation path; keeping it
/// next to `validate_regex_flags` so the supported-flag set lives in
/// one place.
pub(crate) fn build_regex(pattern: &str, flags: &str) -> Result<regex::Regex, regex::Error> {
    let mut builder = regex::RegexBuilder::new(pattern);
    for ch in flags.chars() {
        match ch {
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'u' => {
                builder.unicode(true);
            }
            _ => unreachable!("validate_regex_flags rejects everything else"),
        }
    }
    builder.build()
}
