//! HTTP-engine pagination drivers.
//!
//! Each Pagination variant produces an iterator of "next request" instructions:
//! given the previous response body, decide whether to fetch another page
//! (and how to mutate the request to do so).

use forage_core::ast::Pagination;
use forage_core::{EvalValue, Evaluator, Scope};

use crate::error::{HttpError, HttpResult};

/// Outcome of pagination after consuming a response.
pub enum NextPage {
    /// Stop — accumulated items satisfy the strategy's termination rule.
    Stop,
    /// Continue: mutate the URL with the given query param.
    Continue(Vec<(String, String)>),
}

pub struct PaginationDriver<'a> {
    pub strategy: &'a Pagination,
    pub accumulated: i64,
    pub page: u32,
}

impl<'a> PaginationDriver<'a> {
    pub fn new(strategy: &'a Pagination) -> Self {
        Self {
            strategy,
            accumulated: 0,
            page: 1,
        }
    }

    /// Inspect the response body, decide whether to keep going.
    ///
    /// `scope.current` should be set to the parsed response body before
    /// this is called.
    pub fn advance(&mut self, ev: &Evaluator<'_>, scope: &Scope) -> HttpResult<NextPage> {
        match self.strategy {
            Pagination::PageWithTotal {
                items_path,
                total_path,
                page_param,
                page_size,
                page_zero_indexed,
            } => {
                let items = ev.eval_path(items_path, scope)?;
                let total = ev.eval_path(total_path, scope)?;
                let new_count = match &items {
                    EvalValue::Array(xs) => xs.len() as i64,
                    EvalValue::Null => 0,
                    _ => 0,
                };
                self.accumulated += new_count;
                let total_n = match total {
                    EvalValue::Int(n) => n,
                    EvalValue::Double(n) => n as i64,
                    _ => 0,
                };
                if self.accumulated >= total_n || new_count == 0 {
                    return Ok(NextPage::Stop);
                }
                self.page += 1;
                let n = if *page_zero_indexed {
                    self.page - 1
                } else {
                    self.page
                };
                Ok(NextPage::Continue(vec![
                    (page_param.clone(), n.to_string()),
                    ("pageSize".into(), page_size.to_string()),
                ]))
            }
            Pagination::UntilEmpty {
                items_path,
                page_param,
                page_zero_indexed,
            } => {
                let items = ev.eval_path(items_path, scope)?;
                let count = match &items {
                    EvalValue::Array(xs) => xs.len(),
                    _ => 0,
                };
                if count == 0 {
                    return Ok(NextPage::Stop);
                }
                self.page += 1;
                let n = if *page_zero_indexed {
                    self.page - 1
                } else {
                    self.page
                };
                Ok(NextPage::Continue(vec![(
                    page_param.clone(),
                    n.to_string(),
                )]))
            }
            Pagination::Cursor {
                cursor_path,
                cursor_param,
                ..
            } => {
                let cursor = ev.eval_path(cursor_path, scope)?;
                let next = match cursor {
                    EvalValue::String(s) if !s.is_empty() => s,
                    _ => return Ok(NextPage::Stop),
                };
                Ok(NextPage::Continue(vec![(cursor_param.clone(), next)]))
            }
        }
    }
}

#[allow(dead_code)]
fn unused(_: HttpError) {}
