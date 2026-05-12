//! Auth strategy application: mutates outgoing requests according to the
//! recipe's `auth` block.

use indexmap::IndexMap;

use crate::body::render_body;
use crate::error::{HttpError, HttpResult};
use crate::transport::{HttpRequest, Transport};
use forage_core::ast::{AuthStrategy, BearerLogin, FormLogin, SessionKind};
use forage_core::{EvalValue, Evaluator, Scope};

/// Runtime auth state — what the engine carries from login until the
/// recipe's body finishes. Cookies live in the Transport's cookie jar
/// (reqwest handles them automatically); bearer tokens live here.
#[derive(Debug, Default, Clone)]
pub struct AuthState {
    pub bearer_token: Option<String>,
    pub bearer_header_name: String,
    pub bearer_header_prefix: String,
}

/// Run the session-auth login flow if the recipe declares one. Cookies
/// thread automatically through the Transport's cookie jar; bearer
/// tokens are returned in the AuthState for subsequent header injection.
pub async fn run_session_login(
    auth: Option<&AuthStrategy>,
    transport: &dyn Transport,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    user_agent: &str,
) -> HttpResult<AuthState> {
    let Some(AuthStrategy::Session(session)) = auth else {
        return Ok(AuthState::default());
    };
    match &session.kind {
        SessionKind::FormLogin(fl) => {
            do_form_login(fl, transport, evaluator, scope, user_agent).await?;
            Ok(AuthState::default())
        }
        SessionKind::BearerLogin(bl) => {
            let token = do_bearer_login(bl, transport, evaluator, scope, user_agent).await?;
            Ok(AuthState {
                bearer_token: Some(token),
                bearer_header_name: bl.header_name.clone(),
                bearer_header_prefix: bl.header_prefix.clone(),
            })
        }
        SessionKind::CookiePersist(_) => {
            // CookiePersist loads cookies from a file. For the live Transport
            // we'd hand them to reqwest's cookie store; for replay we ignore.
            // Wire in once the use case lands.
            Ok(AuthState::default())
        }
    }
}

async fn do_form_login(
    fl: &FormLogin,
    transport: &dyn Transport,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    user_agent: &str,
) -> HttpResult<()> {
    let url = evaluator.render_template(&fl.url, scope)?;
    let mut headers: IndexMap<String, String> = IndexMap::new();
    headers.insert("User-Agent".into(), user_agent.into());
    let (content_type, bytes) = render_body(&fl.body, evaluator, scope)?;
    headers.insert("Content-Type".into(), content_type);
    let req = HttpRequest {
        method: fl.method.clone(),
        url,
        headers,
        body: Some(bytes),
    };
    let resp = transport.fetch(req.clone()).await?;
    if !(200..400).contains(&resp.status) {
        return Err(HttpError::Status {
            status: resp.status,
            url: req.url,
        });
    }
    // Cookies are captured automatically by the Transport's cookie jar.
    Ok(())
}

async fn do_bearer_login(
    bl: &BearerLogin,
    transport: &dyn Transport,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    user_agent: &str,
) -> HttpResult<String> {
    let url = evaluator.render_template(&bl.url, scope)?;
    let mut headers: IndexMap<String, String> = IndexMap::new();
    headers.insert("User-Agent".into(), user_agent.into());
    let (content_type, bytes) = render_body(&bl.body, evaluator, scope)?;
    headers.insert("Content-Type".into(), content_type);
    let req = HttpRequest {
        method: bl.method.clone(),
        url,
        headers,
        body: Some(bytes),
    };
    let resp = transport.fetch(req.clone()).await?;
    if !(200..400).contains(&resp.status) {
        return Err(HttpError::Status {
            status: resp.status,
            url: req.url,
        });
    }
    let body_str = resp.body_str();
    let json: serde_json::Value = serde_json::from_str(body_str)
        .map_err(|e| HttpError::Generic(format!("bearerLogin response parse: {e}")))?;
    // Evaluate tokenPath against the response.
    let mut login_scope = scope.clone();
    login_scope.current = Some(EvalValue::from(&json));
    let v = evaluator.eval_path(&bl.token_path, &login_scope)?;
    match v {
        EvalValue::String(s) => Ok(s),
        EvalValue::Null => Err(HttpError::Generic(
            "bearerLogin tokenPath resolved to null".into(),
        )),
        other => Err(HttpError::Generic(format!(
            "bearerLogin tokenPath returned non-string: {other:?}"
        ))),
    }
}

/// Apply auth-derived modifications to the per-request header map.
pub fn apply_request_headers(
    auth: Option<&AuthStrategy>,
    state: &AuthState,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    headers: &mut IndexMap<String, String>,
) -> HttpResult<()> {
    if let Some(AuthStrategy::StaticHeader { name, value }) = auth {
        let rendered = evaluator.render_template(value, scope)?;
        headers.insert(name.clone(), rendered);
    }
    if let Some(token) = &state.bearer_token {
        headers.insert(
            state.bearer_header_name.clone(),
            format!("{}{}", state.bearer_header_prefix, token),
        );
    }
    Ok(())
}
