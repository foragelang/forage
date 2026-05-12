//! HTTP-engine authentication strategies.

use serde::{Deserialize, Serialize};

use crate::ast::expr::{PathExpr, Template};
use crate::ast::http::HTTPBody;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthStrategy {
    /// Static header on every subsequent request.
    StaticHeader { name: String, value: Template },
    /// Prime via HTML page — capture cookies + regex-extract scope variables.
    HtmlPrime {
        step_name: String,
        captured_vars: Vec<HtmlPrimeVar>,
    },
    /// Stateful authenticated session.
    Session(SessionAuth),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HtmlPrimeVar {
    /// `$ajaxNonce` etc. (stored without the leading `$`).
    pub var_name: String,
    pub regex_pattern: String,
    /// 1-based capture group.
    pub group_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionAuth {
    pub kind: SessionKind,
    pub max_reauth_retries: u32,
    /// Cache the session for this many seconds; `None` = no caching.
    pub cache_duration_secs: Option<u64>,
    pub cache_encrypted: bool,
    pub requires_mfa: bool,
    /// Field name for the MFA code on the login body. Default `code`.
    pub mfa_field_name: String,
}

impl Default for SessionAuth {
    fn default() -> Self {
        Self {
            kind: SessionKind::FormLogin(FormLogin::default()),
            max_reauth_retries: 1,
            cache_duration_secs: None,
            cache_encrypted: false,
            requires_mfa: false,
            mfa_field_name: "code".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionKind {
    FormLogin(FormLogin),
    BearerLogin(BearerLogin),
    CookiePersist(CookiePersist),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormLogin {
    pub url: Template,
    pub method: String,
    pub body: HTTPBody,
    pub capture_cookies: bool,
}

impl Default for FormLogin {
    fn default() -> Self {
        Self {
            url: Template::literal(""),
            method: "POST".into(),
            body: HTTPBody::JsonObject(vec![]),
            capture_cookies: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BearerLogin {
    pub url: Template,
    pub method: String,
    pub body: HTTPBody,
    /// e.g. `$.access_token`.
    pub token_path: PathExpr,
    pub header_name: String,
    pub header_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CookiePersist {
    pub source_path: Template,
    pub format: CookieFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CookieFormat {
    /// JSON array `[{"name": ..., "value": ..., "domain": ...}, ...]`.
    Json,
    /// Netscape `cookies.txt` format — tab-separated.
    Netscape,
}
