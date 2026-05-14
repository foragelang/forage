//! Hand-rolled lexer for `.forage` source.
//!
//! Produces `Vec<(Token, Span)>`. Handles `//` line comments, `/* */`
//! block comments, escaped strings, integer/float/bool/null/date literals,
//! and the multi-char operators `\u{2190}` (binding), `\u{2192}` (case arrow),
//! `?.`, `[*]`.
//!
//! Mirrors `Sources/Forage/Parser/Lexer.swift` byte-for-byte semantics so
//! all in-tree recipes tokenize identically.

use thiserror::Error;

use crate::ast::Span;
use crate::parse::token::{Token, is_keyword};

pub fn lex(source: &str) -> Result<Vec<(Token, Span)>, LexError> {
    let mut lx = Lexer::new(source);
    lx.run()?;
    Ok(lx.out)
}

#[derive(Debug, Clone, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character '{ch}' at byte offset {offset}")]
    UnexpectedCharacter { ch: char, offset: usize },
    #[error("unterminated string starting at byte offset {offset}")]
    UnterminatedString { offset: usize },
    #[error("invalid number '{raw}' at byte offset {offset}")]
    InvalidNumber { raw: String, offset: usize },
    #[error("unterminated regex literal starting at byte offset {offset}")]
    UnterminatedRegex { offset: usize },
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    out: Vec<(Token, Span)>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            src: source,
            bytes: source.as_bytes(),
            pos: 0,
            out: Vec::new(),
        }
    }

    fn run(&mut self) -> Result<(), LexError> {
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            if self.pos >= self.bytes.len() {
                break;
            }

            let start = self.pos;
            let c = self.peek_char();
            match c {
                '{' => self.single(Token::LBrace, start),
                '}' => self.single(Token::RBrace, start),
                '(' => self.single(Token::LParen, start),
                ')' => self.single(Token::RParen, start),
                '[' => {
                    if self.starts_with_at(self.pos + 1, "*]") {
                        self.pos += 3;
                        self.out.push((Token::Wildcard, start..self.pos));
                    } else {
                        self.single(Token::LBracket, start);
                    }
                }
                ']' => self.single(Token::RBracket, start),
                ',' => self.single(Token::Comma, start),
                ';' => self.single(Token::Semicolon, start),
                ':' => self.single(Token::Colon, start),
                '.' => self.single(Token::Dot, start),
                '?' => {
                    if self.peek_char_at(1) == '.' {
                        self.pos += 2;
                        self.out.push((Token::QDot, start..self.pos));
                    } else {
                        self.single(Token::Question, start);
                    }
                }
                '|' => self.single(Token::Pipe, start),
                '=' => self.single(Token::Equal, start),
                '>' => self.single(Token::Gt, start),
                '<' => self.single(Token::Lt, start),
                '!' => self.single(Token::Bang, start),
                '+' => self.single(Token::Plus, start),
                '-' => self.single(Token::Dash, start),
                '*' => self.single(Token::Star, start),
                // `/` may begin a comment (`//`, `/*`) or a regex literal
                // `/pattern/flags`. Comments are already eaten by
                // `skip_ws_and_comments`, so reaching this arm with a
                // bare `/` means it's the start of either division (a
                // single `/`) or a regex literal. Disambiguation is
                // context-sensitive: a regex literal only appears in
                // expression position. The lexer can't see syntactic
                // context, so we scan a tentative regex and let the
                // parser interpret it: when the previous token is one
                // that introduces a fresh expression (operator, opener,
                // comma, arrow, `|`, etc.) we emit `RegexLit`; otherwise
                // we emit `Slash` and let the parser handle division.
                '/' => {
                    if self.regex_allowed_here() {
                        self.scan_regex(start)?
                    } else {
                        self.single(Token::Slash, start);
                    }
                }
                '%' => self.single(Token::Percent, start),
                '←' => self.multi(Token::Arrow, start, '←'.len_utf8()),
                '→' => self.multi(Token::CaseArrow, start, '→'.len_utf8()),
                '"' => self.scan_string(start)?,
                '$' => self.scan_dollar(start),
                c if c.is_ascii_digit() => self.scan_number_or_date(start)?,
                c if c.is_ascii_alphabetic() || c == '_' => self.scan_ident_or_keyword(start),
                c => {
                    return Err(LexError::UnexpectedCharacter {
                        ch: c,
                        offset: start,
                    });
                }
            }
        }
        Ok(())
    }

    fn single(&mut self, tok: Token, start: usize) {
        self.pos += 1;
        self.out.push((tok, start..self.pos));
    }

    fn multi(&mut self, tok: Token, start: usize, width: usize) {
        self.pos += width;
        self.out.push((tok, start..self.pos));
    }

    fn scan_string(&mut self, start: usize) -> Result<(), LexError> {
        self.pos += 1; // consume opening quote
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            let c = self.peek_char();
            if c == '"' {
                self.pos += 1;
                self.out.push((Token::Str(s), start..self.pos));
                return Ok(());
            }
            if c == '\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    return Err(LexError::UnterminatedString { offset: start });
                }
                let esc = self.peek_char();
                match esc {
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    other => s.push(other),
                }
                self.pos += esc.len_utf8();
            } else {
                s.push(c);
                self.pos += c.len_utf8();
            }
        }
        Err(LexError::UnterminatedString { offset: start })
    }

    fn scan_dollar(&mut self, start: usize) {
        self.pos += 1; // consume $
        if self.pos < self.bytes.len() {
            let c = self.peek_char();
            if c.is_ascii_alphabetic() || c == '_' {
                let name = self.read_ident();
                let tok = match name.as_str() {
                    "input" => Token::DollarInput,
                    "secret" => Token::DollarSecret,
                    _ => Token::DollarVar(name),
                };
                self.out.push((tok, start..self.pos));
                return;
            }
        }
        // Bare `$` — for current-value paths like `$.foo`.
        self.out.push((Token::DollarRoot, start..self.pos));
    }

    /// Decide whether a bare `/` should start a regex literal vs. a
    /// division operator. Regex literals live in expression position
    /// only — they're meaningful right after an operator, an opener
    /// (`{`, `(`, `[`, `,`), a `←` / `→` / `|`, or at the start of input.
    /// After a value-producing token (identifier, number, `]`, `)`, `}`)
    /// a `/` is binary division.
    fn regex_allowed_here(&self) -> bool {
        let Some((prev, _)) = self.out.last() else {
            return true;
        };
        match prev {
            // Value-producing tokens — `/` is division.
            Token::Ident(_)
            | Token::TypeName(_)
            | Token::Str(_)
            | Token::Int(_)
            | Token::Float(_)
            | Token::Bool(_)
            | Token::Null
            | Token::Date { .. }
            | Token::RegexLit { .. }
            | Token::DollarRoot
            | Token::DollarInput
            | Token::DollarSecret
            | Token::DollarVar(_)
            | Token::RParen
            | Token::RBracket
            | Token::RBrace
            | Token::Wildcard => false,
            // Everything else — operators, openers, punctuation — opens
            // a fresh expression slot.
            _ => true,
        }
    }

    /// `/pattern/flags` — scan until the next unescaped `/` and trailing
    /// ASCII flag letters. `\` escapes any following character (including
    /// `/`) so authors can match `/`, newlines (`\n`), digits (`\d`), etc.
    /// The pattern itself is not interpreted here; the parser hands it
    /// to the `regex` crate.
    fn scan_regex(&mut self, start: usize) -> Result<(), LexError> {
        self.pos += 1; // opening `/`
        let mut pattern = String::new();
        let mut closed = false;
        while self.pos < self.bytes.len() {
            let c = self.peek_char();
            if c == '\n' {
                break;
            }
            if c == '\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    break;
                }
                let esc = self.peek_char();
                pattern.push('\\');
                pattern.push(esc);
                self.pos += esc.len_utf8();
                continue;
            }
            if c == '/' {
                self.pos += 1;
                closed = true;
                break;
            }
            pattern.push(c);
            self.pos += c.len_utf8();
        }
        if !closed {
            return Err(LexError::UnterminatedRegex { offset: start });
        }
        let mut flags = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c.is_ascii_alphabetic() {
                flags.push(c as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        self.out
            .push((Token::RegexLit { pattern, flags }, start..self.pos));
        Ok(())
    }

    fn scan_number_or_date(&mut self, start: usize) -> Result<(), LexError> {
        // `-N` no longer enters here — `-` always lexes as `Dash` so that
        // `x - 7` reads as `Variable($x) - Int(7)`. Unary minus is the
        // parser's job; folding `-7` back into a single `Int(-7)` token
        // would re-introduce the binary/unary ambiguity at every binary
        // call site.
        let mut end = self.pos;
        let int_start = end;
        while end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
            end += 1;
        }
        let int_str = &self.src[int_start..end];

        // YYYY-MM-DD?
        if end < self.bytes.len() && self.bytes[end] == b'-' && int_str.len() == 4 {
            let mid = end + 1;
            let mut m_end = mid;
            while m_end < self.bytes.len() && self.bytes[m_end].is_ascii_digit() {
                m_end += 1;
            }
            if m_end - mid == 2 && m_end < self.bytes.len() && self.bytes[m_end] == b'-' {
                let day_start = m_end + 1;
                let mut day_end = day_start;
                while day_end < self.bytes.len() && self.bytes[day_end].is_ascii_digit() {
                    day_end += 1;
                }
                if day_end - day_start == 2 {
                    let y: i32 = int_str.parse().expect("4 digits");
                    let m: u32 = self.src[mid..m_end].parse().expect("2 digits");
                    let d: u32 = self.src[day_start..day_end].parse().expect("2 digits");
                    self.pos = day_end;
                    self.out.push((
                        Token::Date {
                            year: y,
                            month: m,
                            day: d,
                        },
                        start..day_end,
                    ));
                    return Ok(());
                }
            }
        }

        // Decimal?
        if end < self.bytes.len()
            && self.bytes[end] == b'.'
            && end + 1 < self.bytes.len()
            && self.bytes[end + 1].is_ascii_digit()
        {
            end += 1;
            while end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
                end += 1;
            }
            let raw = &self.src[start..end];
            let parsed: f64 = raw.parse().map_err(|_| LexError::InvalidNumber {
                raw: raw.into(),
                offset: start,
            })?;
            self.pos = end;
            self.out.push((Token::Float(parsed), start..end));
            return Ok(());
        }

        let raw = &self.src[start..end];
        let parsed: i64 = raw.parse().map_err(|_| LexError::InvalidNumber {
            raw: raw.into(),
            offset: start,
        })?;
        self.pos = end;
        self.out.push((Token::Int(parsed), start..end));
        Ok(())
    }

    fn scan_ident_or_keyword(&mut self, start: usize) {
        let name = self.read_ident();
        let span = start..self.pos;
        let tok = if name == "true" {
            Token::Bool(true)
        } else if name == "false" {
            Token::Bool(false)
        } else if name == "null" {
            Token::Null
        } else if is_keyword(&name) {
            Token::Keyword(name)
        } else if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            Token::TypeName(name)
        } else {
            Token::Ident(name)
        };
        self.out.push((tok, span));
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos + 1 < self.bytes.len() && &self.bytes[self.pos..self.pos + 2] == b"//" {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < self.bytes.len() && &self.bytes[self.pos..self.pos + 2] == b"/*" {
                self.pos += 2;
                while self.pos + 1 < self.bytes.len()
                    && &self.bytes[self.pos..self.pos + 2] != b"*/"
                {
                    self.pos += 1;
                }
                if self.pos + 1 < self.bytes.len() {
                    self.pos += 2;
                }
                continue;
            }
            break;
        }
    }

    fn peek_char(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    fn peek_char_at(&self, offset: usize) -> char {
        let mut chars = self.src[self.pos..].chars();
        for _ in 0..offset {
            if chars.next().is_none() {
                return '\0';
            }
        }
        chars.next().unwrap_or('\0')
    }

    fn starts_with_at(&self, at: usize, s: &str) -> bool {
        self.src.get(at..at + s.len()).is_some_and(|x| x == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_only(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn simple_punctuation() {
        let t = tokens_only("{ } ( ) [ ] , ; : .");
        assert_eq!(
            t,
            vec![
                Token::LBrace,
                Token::RBrace,
                Token::LParen,
                Token::RParen,
                Token::LBracket,
                Token::RBracket,
                Token::Comma,
                Token::Semicolon,
                Token::Colon,
                Token::Dot,
            ]
        );
    }

    #[test]
    fn wildcard_and_qdot() {
        let t = tokens_only("[*] ?.");
        assert_eq!(t, vec![Token::Wildcard, Token::QDot]);
    }

    #[test]
    fn arrows() {
        let t = tokens_only("\u{2190} \u{2192}");
        assert_eq!(t, vec![Token::Arrow, Token::CaseArrow]);
    }

    #[test]
    fn string_with_escapes() {
        let t = tokens_only(r#""hello \"world\" \n""#);
        assert_eq!(t, vec![Token::Str("hello \"world\" \n".into())]);
    }

    #[test]
    fn numbers_and_dates() {
        // Bare `-` is always its own token; folding `-7` into a single
        // `Int(-7)` would re-introduce binary/unary ambiguity (`x - 7`
        // is subtraction, not `x` followed by `-7`).
        let t = tokens_only("42 -7 2.5 1990-01-01");
        assert_eq!(
            t,
            vec![
                Token::Int(42),
                Token::Dash,
                Token::Int(7),
                Token::Float(2.5),
                Token::Date {
                    year: 1990,
                    month: 1,
                    day: 1
                },
            ]
        );
    }

    #[test]
    fn arithmetic_operators_tokenize() {
        let t = tokens_only("1 + 2 - 3 * 4 / 5 % 6");
        assert_eq!(
            t,
            vec![
                Token::Int(1),
                Token::Plus,
                Token::Int(2),
                Token::Dash,
                Token::Int(3),
                Token::Star,
                Token::Int(4),
                Token::Slash,
                Token::Int(5),
                Token::Percent,
                Token::Int(6),
            ],
        );
    }

    #[test]
    fn regex_literal_in_expression_position() {
        // After `|`, `/` opens a regex literal (the consumer transform
        // takes a regex value); the trailing flags are preserved.
        let t = tokens_only("$s | match(/(\\d+)\\s*(oz|g)/i)");
        assert!(
            matches!(
                t.iter().find(|tok| matches!(tok, Token::RegexLit { .. })),
                Some(Token::RegexLit { pattern, flags })
                    if pattern == "(\\d+)\\s*(oz|g)" && flags == "i",
            ),
            "got {t:?}",
        );
    }

    #[test]
    fn slash_after_value_is_division() {
        // After an identifier or `)`, `/` reads as division — not a
        // regex literal.
        let t = tokens_only("$a / 2");
        assert_eq!(
            t,
            vec![Token::DollarVar("a".into()), Token::Slash, Token::Int(2)],
        );
    }

    #[test]
    fn idents_and_keywords() {
        let t = tokens_only("recipe foo Bar baz_qux");
        assert_eq!(
            t,
            vec![
                Token::Keyword("recipe".into()),
                Token::Ident("foo".into()),
                Token::TypeName("Bar".into()),
                Token::Ident("baz_qux".into()),
            ]
        );
    }

    #[test]
    fn dollar_forms() {
        let t = tokens_only("$ $input $secret $name");
        assert_eq!(
            t,
            vec![
                Token::DollarRoot,
                Token::DollarInput,
                Token::DollarSecret,
                Token::DollarVar("name".into()),
            ]
        );
    }

    #[test]
    fn comments() {
        let t = tokens_only("// line comment\n42 /* block */ 7");
        assert_eq!(t, vec![Token::Int(42), Token::Int(7)]);
    }

    #[test]
    fn bool_and_null() {
        let t = tokens_only("true false null");
        assert_eq!(t, vec![Token::Bool(true), Token::Bool(false), Token::Null]);
    }
}
