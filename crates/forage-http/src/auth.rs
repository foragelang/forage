//! Auth strategy application: mutates outgoing requests according to the
//! recipe's `auth` block.
//!
//! Static-header auth lands in this commit; htmlPrime + session.* land
//! incrementally in subsequent R2 deliverables.

use indexmap::IndexMap;

use crate::error::HttpResult;
use forage_core::ast::AuthStrategy;
use forage_core::{Evaluator, Scope};

/// Apply auth-derived modifications to the per-request header map.
pub fn apply_request_headers(
    auth: Option<&AuthStrategy>,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    headers: &mut IndexMap<String, String>,
) -> HttpResult<()> {
    if let Some(AuthStrategy::StaticHeader { name, value }) = auth {
        let rendered = evaluator.render_template(value, scope)?;
        headers.insert(name.clone(), rendered);
    }
    Ok(())
}
