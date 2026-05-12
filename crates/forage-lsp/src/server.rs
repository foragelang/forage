//! tower-lsp server implementing the Forage LSP surface.

use std::sync::Arc;

use forage_core::ast::FieldType;
use forage_core::validate::BUILTIN_TRANSFORMS;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::docstore::DocStore;

pub struct ForageLsp {
    client: Client,
    store: Arc<DocStore>,
}

impl ForageLsp {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: Arc::new(DocStore::new()),
        }
    }
}

const KEYWORDS: &[&str] = &[
    "recipe",
    "engine",
    "http",
    "browser",
    "type",
    "enum",
    "input",
    "secret",
    "step",
    "method",
    "url",
    "headers",
    "body",
    "json",
    "form",
    "raw",
    "auth",
    "staticHeader",
    "htmlPrime",
    "session",
    "formLogin",
    "bearerLogin",
    "cookiePersist",
    "extract",
    "regex",
    "groups",
    "paginate",
    "pageWithTotal",
    "untilEmpty",
    "cursor",
    "for",
    "in",
    "emit",
    "case",
    "of",
    "expect",
    "observe",
    "browserPaginate",
    "scroll",
    "ageGate",
    "autoFill",
    "captures",
    "match",
    "document",
    "interactive",
    "bootstrapURL",
    "cookieDomains",
    "sessionExpiredPattern",
];

#[tower_lsp::async_trait]
impl LanguageServer for ForageLsp {
    async fn initialize(
        &self,
        _: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "forage-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["$".into(), ".".into(), "|".into()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "forage-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let diags = self.store.upsert(uri.clone(), params.text_document.text);
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // FULL sync — the change contains the entire new document.
        let Some(change) = params.content_changes.into_iter().next() else {
            return;
        };
        let diags = self.store.upsert(uri.clone(), change.text);
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.store.remove(&params.text_document.uri);
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let mut items: Vec<CompletionItem> = Vec::new();

        // Keywords.
        for kw in KEYWORDS {
            items.push(CompletionItem {
                label: (*kw).into(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }
        // Transforms.
        for t in BUILTIN_TRANSFORMS {
            items.push(CompletionItem {
                label: (*t).into(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some("transform".into()),
                ..Default::default()
            });
        }
        // Inputs / secrets / types from the parsed recipe (if available).
        self.store.with(&uri, |doc| {
            if let Some(r) = &doc.recipe {
                for inp in &r.inputs {
                    items.push(CompletionItem {
                        label: format!("$input.{}", inp.name),
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some(format!("input — {}", field_type_label(&inp.ty))),
                        ..Default::default()
                    });
                }
                for s in &r.secrets {
                    items.push(CompletionItem {
                        label: format!("$secret.{s}"),
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some("secret".into()),
                        ..Default::default()
                    });
                }
                for ty in &r.types {
                    items.push(CompletionItem {
                        label: ty.name.clone(),
                        kind: Some(CompletionItemKind::STRUCT),
                        detail: Some("recipe type".into()),
                        ..Default::default()
                    });
                }
                for en in &r.enums {
                    items.push(CompletionItem {
                        label: en.name.clone(),
                        kind: Some(CompletionItemKind::ENUM),
                        detail: Some(format!("enum [{}]", en.variants.join(", "))),
                        ..Default::default()
                    });
                }
            }
        });
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let response = self.store.with(&uri, |doc| {
            // Find the word at `pos`.
            let line_str = doc.source.lines().nth(pos.line as usize)?;
            let col = pos.character as usize;
            let bytes = line_str.as_bytes();
            if col > bytes.len() {
                return None;
            }
            let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
            let mut s = col;
            while s > 0 && is_word(bytes[s - 1]) {
                s -= 1;
            }
            let mut e = col;
            while e < bytes.len() && is_word(bytes[e]) {
                e += 1;
            }
            let word = std::str::from_utf8(&bytes[s..e]).ok()?.to_string();
            if word.is_empty() {
                return None;
            }
            // Match against transforms, keywords, recipe symbols.
            if BUILTIN_TRANSFORMS.contains(&word.as_str()) {
                return Some(Hover {
                    contents: HoverContents::Scalar(MarkedString::String(format!(
                        "**{word}** — built-in transform"
                    ))),
                    range: None,
                });
            }
            if let Some(r) = &doc.recipe {
                if let Some(ty) = r.types.iter().find(|t| t.name == word) {
                    let fields: Vec<String> = ty
                        .fields
                        .iter()
                        .map(|f| {
                            format!(
                                "{}: {}{}",
                                f.name,
                                field_type_label(&f.ty),
                                if f.optional { "?" } else { "" }
                            )
                        })
                        .collect();
                    return Some(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(format!(
                            "**{}**\n\n{{ {} }}",
                            ty.name,
                            fields.join(", ")
                        ))),
                        range: None,
                    });
                }
                if let Some(inp) = r.inputs.iter().find(|i| i.name == word) {
                    return Some(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(format!(
                            "**input {}** — {}",
                            inp.name,
                            field_type_label(&inp.ty)
                        ))),
                        range: None,
                    });
                }
                if let Some(en) = r.enums.iter().find(|e| e.name == word) {
                    return Some(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(format!(
                            "**enum {}** {{ {} }}",
                            en.name,
                            en.variants.join(" | ")
                        ))),
                        range: None,
                    });
                }
            }
            None
        });
        Ok(response.flatten())
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let symbols = self.store.with(&uri, |doc| {
            let mut out: Vec<SymbolInformation> = Vec::new();
            let Some(r) = &doc.recipe else {
                return out;
            };
            #[allow(deprecated)]
            for ty in &r.types {
                out.push(SymbolInformation {
                    name: ty.name.clone(),
                    kind: SymbolKind::STRUCT,
                    location: Location {
                        uri: uri.clone(),
                        range: doc.line_map.range_for(0..0),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(r.name.clone()),
                });
            }
            #[allow(deprecated)]
            for en in &r.enums {
                out.push(SymbolInformation {
                    name: en.name.clone(),
                    kind: SymbolKind::ENUM,
                    location: Location {
                        uri: uri.clone(),
                        range: doc.line_map.range_for(0..0),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(r.name.clone()),
                });
            }
            #[allow(deprecated)]
            for inp in &r.inputs {
                out.push(SymbolInformation {
                    name: format!("input {}", inp.name),
                    kind: SymbolKind::VARIABLE,
                    location: Location {
                        uri: uri.clone(),
                        range: doc.line_map.range_for(0..0),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(r.name.clone()),
                });
            }
            out
        });
        Ok(symbols.map(DocumentSymbolResponse::Flat))
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }
}

fn field_type_label(t: &FieldType) -> String {
    match t {
        FieldType::String => "String".into(),
        FieldType::Int => "Int".into(),
        FieldType::Double => "Double".into(),
        FieldType::Bool => "Bool".into(),
        FieldType::Array(inner) => format!("[{}]", field_type_label(inner)),
        FieldType::Record(n) => n.clone(),
        FieldType::EnumRef(n) => n.clone(),
    }
}

/// Spawn the LSP server on stdio. Used by `forage lsp` and Studio's
/// child process.
pub async fn run_stdio() {
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let (service, socket) = tower_lsp::LspService::new(ForageLsp::new);
    tower_lsp::Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}
