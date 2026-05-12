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
}

/// Top-level entry: lex + parse a complete recipe.
pub fn parse(source: &str) -> Result<Recipe, ParseError> {
    let toks = lex(source)?;
    let mut p = Parser::new(toks, source.len());
    p.parse_recipe()
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
        match self.peek().cloned() {
            Some(Token::Int(n)) => {
                self.pos += 1;
                Ok(n)
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

    /// recipe := imports? recipe_decl
    fn parse_recipe(&mut self) -> Result<Recipe, ParseError> {
        let mut imports = Vec::new();
        while self.eat_keyword("import") {
            let r = self.parse_ref()?;
            imports.push(r);
        }

        self.expect_keyword("recipe")?;
        let name = self.expect_string()?;
        self.expect_punct(&Token::LBrace)?;

        // engine <kind>
        self.expect_keyword("engine")?;
        let engine_kind = self.parse_engine_kind()?;

        let mut types = Vec::new();
        let mut enums = Vec::new();
        let mut inputs = Vec::new();
        let mut secrets = Vec::new();
        let mut auth: Option<AuthStrategy> = None;
        let mut body: Vec<Statement> = Vec::new();
        let mut browser: Option<BrowserConfig> = None;
        let mut expectations: Vec<Expectation> = Vec::new();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            match self.peek().cloned() {
                Some(Token::Keyword(k)) => match k.as_str() {
                    "type" => types.push(self.parse_type_decl()?),
                    "enum" => enums.push(self.parse_enum_decl()?),
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
                    "step" | "for" | "emit" => {
                        body.push(self.parse_statement()?);
                    }
                    other => return Err(self.generic(&format!("unexpected keyword '{other}'"))),
                },
                Some(other) => {
                    return Err(self.generic(&format!(
                        "unexpected token in recipe body: {}",
                        other.describe()
                    )));
                }
                None => break,
            }
        }
        self.expect_punct(&Token::RBrace)?;

        Ok(Recipe {
            name,
            engine_kind,
            types,
            enums,
            inputs,
            auth,
            body,
            browser,
            expectations,
            imports,
            secrets,
        })
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

    fn parse_ref(&mut self) -> Result<HubRecipeRef, ParseError> {
        let raw = match self.peek().cloned() {
            Some(Token::Ref(s)) => {
                self.bump();
                s
            }
            _ => return Err(self.unexpected("recipe reference")),
        };
        // Parse `author/slug` or `slug` or `author/slug@v3`. Simple v1 — we
        // accept any non-whitespace and split.
        let (refname, version) = if let Some((before, after)) = raw.split_once('@') {
            let v: u32 = after.trim_start_matches('v').parse().unwrap_or(0);
            (before.to_string(), Some(v))
        } else {
            (raw, None)
        };
        let (author, slug) = match refname.split_once('/') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => ("forage".to_string(), refname),
        };
        Ok(HubRecipeRef {
            author,
            slug,
            version,
        })
    }

    // --- type / enum / input ----------------------------------------------

    /// type_decl := 'type' TypeName '{' field (';'|',')? ... '}'
    fn parse_type_decl(&mut self) -> Result<RecipeType, ParseError> {
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
        Ok(RecipeType { name, fields })
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
            Some(Token::TypeName(t)) => {
                self.bump();
                // Without resolution, treat any user-declared TypeName as a Record.
                // Validator distinguishes record vs enum by lookup.
                Ok(FieldType::Record(t))
            }
            _ => Err(self.unexpected("type")),
        }
    }

    fn parse_enum_decl(&mut self) -> Result<RecipeEnum, ParseError> {
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
        Ok(RecipeEnum { name, variants })
    }

    fn parse_input_decl(&mut self) -> Result<InputDecl, ParseError> {
        self.expect_keyword("input")?;
        let name = self.expect_field_name()?;
        self.expect_punct(&Token::Colon)?;
        let ty = self.parse_field_type()?;
        let optional = self.eat_punct(&Token::Question);
        Ok(InputDecl { name, ty, optional })
    }

    // --- expressions -------------------------------------------------------

    /// extraction := primary ('|' transform_call)*
    fn parse_extraction(&mut self) -> Result<ExtractionExpr, ParseError> {
        let head = self.parse_primary_expr()?;
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

    fn parse_transform_call(&mut self) -> Result<TransformCall, ParseError> {
        let name = self.expect_ident()?;
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
        })
    }

    fn parse_emit(&mut self) -> Result<Emission, ParseError> {
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
        Ok(Emission {
            type_name,
            bindings,
        })
    }

    fn parse_step(&mut self) -> Result<HTTPStep, ParseError> {
        self.expect_keyword("step")?;
        let name = self.expect_ident()?;
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
        let _var = match self.peek().cloned() {
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
            iter_path,
            body,
        })
    }

    fn parse_capture_document_body(&mut self) -> Result<DocumentCaptureRule, ParseError> {
        self.expect_keyword("for")?;
        let _var = match self.peek().cloned() {
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
        Ok(DocumentCaptureRule { iter_path, body })
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
