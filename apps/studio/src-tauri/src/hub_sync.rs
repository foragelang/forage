//! Tauri-side hub sync glue.
//!
//! Studio's Tauri commands stay thin: they pull auth tokens off the
//! local store, build a [`HubClient`], and hand off to the shared
//! [`forage_hub::operations`] surface. The structured publish error
//! lives here because it's specific to Studio's UI affordance — the
//! "you're behind v{N}; refresh and retry" banner with a diff link
//! that Studio renders when the server returns 409 stale-base.

use std::path::Path;
use std::sync::LazyLock;

use serde::Serialize;
use ts_rs::TS;

use forage_hub::{
    AuthStore, ForageMeta, HubClient, HubError, PublishResponse, SyncOutcome,
    assemble_publish_request, fork_from_hub, publish_from_workspace, sync_from_hub,
};

/// Typed error surface for `publish_recipe`. The stale-base variant
/// carries the version numbers the UI needs to render the rebase
/// prompt; other failures fall through `Other` with a message.
///
/// Tauri serializes this as the JS-side discriminated union exported
/// through ts-rs.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublishError {
    /// `base_version` is behind `latest_version`. The UI renders a
    /// rebase prompt that links to the hub IDE's diff view.
    StaleBase {
        latest_version: u32,
        your_base: Option<u32>,
        message: String,
    },
    /// Caller hit the publish path without a signed-in account.
    NotSignedIn { message: String },
    /// The server responded with a body that doesn't match the
    /// documented error envelope. Different from `Other` because the
    /// UI can render a "report-this-bug" affordance: a malformed
    /// envelope means the hub itself is broken, not the caller.
    ServerMalformed { detail: String },
    /// Anything else — parse failure, transport error, server 5xx.
    Other { message: String },
}

impl PublishError {
    pub fn from_hub_error(e: HubError) -> Self {
        match e {
            HubError::StaleBase {
                latest_version,
                your_base,
                message,
            } => PublishError::StaleBase {
                latest_version,
                your_base,
                message,
            },
            // A server-issued 401 means the token is expired/revoked;
            // the UI's affordance is the same as "no local token"
            // (re-banner the sign-in flow), so collapse both into
            // NotSignedIn rather than letting a stale token fall
            // through to a generic toast.
            HubError::Api { status: 401, message, .. } => PublishError::NotSignedIn { message },
            // A malformed error envelope is a hub bug, not a caller
            // bug — surface it under its own discriminant so the UI
            // can render the right affordance.
            HubError::ServerMalformed { detail } => PublishError::ServerMalformed { detail },
            other => PublishError::Other {
                message: other.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PublishOutcome {
    pub author: String,
    pub slug: String,
    pub version: u32,
    pub latest_version: u32,
}

impl From<PublishResponse> for PublishOutcome {
    fn from(r: PublishResponse) -> Self {
        Self {
            author: r.author,
            slug: r.slug,
            version: r.version,
            latest_version: r.latest_version,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SyncOutcomeWire {
    pub author: String,
    pub slug: String,
    pub version: u32,
    pub origin: String,
}

impl From<SyncOutcome> for SyncOutcomeWire {
    fn from(o: SyncOutcome) -> Self {
        let ForageMeta {
            origin,
            author,
            slug,
            base_version: _,
            forked_from: _,
        } = o.meta;
        Self {
            author,
            slug,
            version: o.version,
            origin,
        }
    }
}

/// Pull a stored bearer token for `hub_url`. Returns `None` when the
/// user isn't signed in; callers surface this with a `NotSignedIn`
/// error rather than swallowing it.
pub fn bearer_for(hub_url: &str) -> Option<String> {
    let host = host_of(hub_url);
    AuthStore::new()
        .read(&host)
        .ok()
        .flatten()
        .map(|t| t.access_token)
}

pub fn client_for(hub_url: &str) -> (HubClient, Option<String>) {
    let token = bearer_for(hub_url);
    let mut client = HubClient::new(hub_url);
    if let Some(t) = token.clone() {
        client = client.with_token(t);
    }
    (client, token)
}

pub async fn run_sync(
    workspace_root: &Path,
    hub_url: &str,
    author: &str,
    slug: &str,
    version: Option<u32>,
) -> Result<SyncOutcomeWire, PublishError> {
    // Sync doesn't require a signed-in caller — the hub serves public
    // packages without auth. We still attach the bearer when present
    // so the server can rate-limit by user instead of by IP.
    let (client, _token) = client_for(hub_url);
    sync_from_hub(&client, workspace_root, author, slug, version)
        .await
        .map(SyncOutcomeWire::from)
        .map_err(PublishError::from_hub_error)
}

pub async fn run_fork(
    workspace_root: &Path,
    hub_url: &str,
    upstream_author: &str,
    upstream_slug: &str,
    r#as: Option<String>,
) -> Result<SyncOutcomeWire, PublishError> {
    let (client, token) = client_for(hub_url);
    if token.is_none() {
        return Err(PublishError::NotSignedIn {
            message: "sign in to fork a recipe".into(),
        });
    }
    fork_from_hub(&client, workspace_root, upstream_author, upstream_slug, r#as)
        .await
        .map(SyncOutcomeWire::from)
        .map_err(PublishError::from_hub_error)
}

pub async fn run_publish(
    workspace_root: &Path,
    hub_url: &str,
    author: &str,
    slug: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> Result<PublishOutcome, PublishError> {
    let (client, token) = client_for(hub_url);
    if token.is_none() {
        return Err(PublishError::NotSignedIn {
            message: "sign in to publish a recipe".into(),
        });
    }
    publish_from_workspace(
        &client,
        workspace_root,
        author,
        slug,
        description,
        category,
        tags,
    )
    .await
    .map(PublishOutcome::from)
    .map_err(PublishError::from_hub_error)
}

/// Dry-run preview: assemble the artifact but don't POST. Returns the
/// byte count + base version the user would be publishing against, so
/// the UI's pre-publish confirmation can show what's about to go up.
pub fn preview_publish(
    workspace_root: &Path,
    slug: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> Result<PublishPreview, PublishError> {
    let req = assemble_publish_request(workspace_root, slug, description, category, tags)
        .map_err(PublishError::from_hub_error)?;
    let bytes = req.recipe.len()
        + req.decls.iter().map(|d| d.source.len()).sum::<usize>()
        + req.fixtures.iter().map(|f| f.content.len()).sum::<usize>();
    Ok(PublishPreview {
        recipe_bytes: bytes as u64,
        base_version: req.base_version,
        decls_count: req.decls.len() as u32,
        fixtures_count: req.fixtures.len() as u32,
    })
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PublishPreview {
    pub recipe_bytes: u64,
    pub base_version: Option<u32>,
    pub decls_count: u32,
    pub fixtures_count: u32,
}

/// Author + slug regex — matches the hub's SEGMENT_RE. Compiled
/// once; the deeplink handler can fire repeatedly so we don't want
/// per-call regex construction in the hot path.
static SEGMENT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[a-z0-9][a-z0-9-]{0,38}$").expect("static SEGMENT_RE must compile")
});

/// Validate a `(author, slug)` pair against the regex the hub uses.
/// The deeplink handler runs this before passing the values into
/// `sync_from_hub` so an opportunistic URL can't smuggle a `..` or a
/// shell escape through.
pub fn validate_segments(author: &str, slug: &str) -> Result<(), String> {
    if !SEGMENT_RE.is_match(author) {
        return Err(format!("invalid author segment: {author}"));
    }
    if !SEGMENT_RE.is_match(slug) {
        return Err(format!("invalid slug segment: {slug}"));
    }
    Ok(())
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split("//").nth(1).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

/// Parse a `forage://` deeplink. Accepts:
///
/// - `forage://clone/<author>/<slug>` (latest)
/// - `forage://clone/<author>/<slug>?version=N`
///
/// Returns `(author, slug, version)`. Strict — anything that doesn't
/// match the regex / shape is an error rather than a best-effort
/// guess. The handler in `lib.rs` validates the regex before passing
/// the result to `sync_from_hub`.
pub fn parse_clone_url(url: &str) -> Result<(String, String, Option<u32>), String> {
    let after_scheme = url
        .strip_prefix("forage://")
        .ok_or_else(|| format!("not a forage:// URL: {url}"))?;
    let (path, query) = match after_scheme.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (after_scheme, None),
    };
    let mut parts = path.trim_matches('/').splitn(3, '/');
    let kind = parts.next().unwrap_or("");
    if kind != "clone" {
        return Err(format!("unknown forage:// action: {kind}"));
    }
    let author = parts
        .next()
        .ok_or_else(|| "missing author in forage://clone URL".to_string())?
        .to_string();
    let slug = parts
        .next()
        .ok_or_else(|| "missing slug in forage://clone URL".to_string())?
        .to_string();
    if author.is_empty() || slug.is_empty() {
        return Err("empty author or slug in forage://clone URL".into());
    }
    validate_segments(&author, &slug)?;
    let version = match query {
        Some(q) => parse_version_query(q)?,
        None => None,
    };
    Ok((author, slug, version))
}

fn parse_version_query(q: &str) -> Result<Option<u32>, String> {
    for pair in q.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = kv.next().unwrap_or("");
        let v = kv.next().unwrap_or("");
        if k == "version" {
            let n: u32 = v
                .parse()
                .map_err(|_| format!("version must be a positive integer, got {v:?}"))?;
            if n == 0 {
                return Err("version must be >= 1".into());
            }
            return Ok(Some(n));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clone_url_latest() {
        let (a, s, v) = parse_clone_url("forage://clone/alice/zen-leaf").unwrap();
        assert_eq!(a, "alice");
        assert_eq!(s, "zen-leaf");
        assert!(v.is_none());
    }

    #[test]
    fn parse_clone_url_with_version() {
        let (a, s, v) = parse_clone_url("forage://clone/alice/zen-leaf?version=3").unwrap();
        assert_eq!(a, "alice");
        assert_eq!(s, "zen-leaf");
        assert_eq!(v, Some(3));
    }

    #[test]
    fn parse_clone_url_rejects_bad_segments() {
        for bad in [
            "forage://clone/Alice/zen-leaf",      // uppercase
            "forage://clone/../zen-leaf",         // traversal
            "forage://clone/alice/",              // empty slug
            "forage://clone/alice",               // missing slug
            "forage://other/alice/zen-leaf",      // unknown verb
            "https://example.com/alice/zen-leaf", // wrong scheme
        ] {
            assert!(parse_clone_url(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn parse_clone_url_rejects_zero_version() {
        assert!(parse_clone_url("forage://clone/alice/zen-leaf?version=0").is_err());
        assert!(parse_clone_url("forage://clone/alice/zen-leaf?version=abc").is_err());
    }

    /// A server-issued 401 (expired/revoked token) must surface as
    /// `NotSignedIn` so the UI rebanners the sign-in flow. A 403 or
    /// any other API error stays generic — only 401 means "you need
    /// to authenticate."
    #[test]
    fn server_401_maps_to_not_signed_in() {
        let mapped = PublishError::from_hub_error(HubError::Api {
            status: 401,
            code: "unauthenticated".into(),
            message: "token expired".into(),
        });
        match mapped {
            PublishError::NotSignedIn { message } => assert_eq!(message, "token expired"),
            other => panic!("expected NotSignedIn, got {other:?}"),
        }

        let other = PublishError::from_hub_error(HubError::Api {
            status: 403,
            code: "forbidden".into(),
            message: "no permission".into(),
        });
        assert!(matches!(other, PublishError::Other { .. }));
    }
}
