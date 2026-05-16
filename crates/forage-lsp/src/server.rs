//! tower-lsp server implementing the Forage LSP surface.

use std::sync::Arc;

use forage_core::ast::FieldType;
use forage_core::parse::KEYWORDS;
use forage_core::validate::BUILTIN_TRANSFORMS;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::docstore::{DocStore, workspace_root_for};
use crate::offsets::lsp_range;

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
        // Ask the client to watch every `.forage` file and `forage.toml`
        // across the open workspaces so the catalog stays fresh when the
        // user edits a sibling file outside the editor. The client
        // routes change notifications to `did_change_watched_files`,
        // which `refresh_workspace`s and republishes diagnostics.
        let watchers = vec![
            FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.forage".into()),
                kind: None,
            },
            FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/forage.toml".into()),
                kind: None,
            },
        ];
        let registration = Registration {
            id: "forage-lsp/workspace-watcher".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions { watchers })
                    .expect("serialize watcher options"),
            ),
        };
        if let Err(e) = self.client.register_capability(vec![registration]).await {
            tracing::warn!(error = %e, "client refused workspace file watch registration");
        }
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
        self.client
            .publish_diagnostics(uri.clone(), diags, None)
            .await;

        // Fan out to every other open doc in the same workspace. Any
        // edit can shift cross-file state: a declarations-file change
        // moves the catalog every recipe sees, and a recipe-file change
        // can add or remove a `share` declaration that affects sibling
        // cross-file diagnostics. The upserted doc already received its
        // own diagnostics; siblings need a fresh pass.
        if self.store.is_in_workspace(&uri)
            && let Some(root) = workspace_root_for(&uri)
        {
            let refreshed = self.store.refresh_workspace(&root);
            for (other_uri, diags) in refreshed {
                if other_uri == uri {
                    continue;
                }
                self.client
                    .publish_diagnostics(other_uri, diags, None)
                    .await;
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.store.remove(&params.text_document.uri);
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // External edits to a `forage.toml` or a `.forage` file inside
        // a workspace can shift the catalog for every recipe in that
        // workspace. Refresh every affected workspace once and republish
        // diagnostics for the recipes that live in it.
        use std::collections::HashSet;
        let mut roots: HashSet<std::path::PathBuf> = HashSet::new();
        for change in &params.changes {
            if let Some(root) = workspace_root_for(&change.uri) {
                roots.insert(root);
            }
        }
        for root in roots {
            let refreshed = self.store.refresh_workspace(&root);
            for (uri, diags) in refreshed {
                self.client.publish_diagnostics(uri, diags, None).await;
            }
        }
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
            if let Some(r) = &doc.file {
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
        // Delegate to the shared `intel::hover_at` so Studio (which
        // calls intel directly via Tauri) and LSP-protocol clients
        // (which take this path) always agree on hover content.
        let info = self
            .store
            .with(&uri, |doc| {
                crate::intel::hover_at(&doc.source, pos.line, pos.character)
            })
            .flatten();
        Ok(info.map(|h| Hover {
            contents: HoverContents::Scalar(MarkedString::String(h.markdown)),
            range: None,
        }))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let symbols = self.store.with(&uri, |doc| {
            let mut out: Vec<SymbolInformation> = Vec::new();
            let Some(r) = &doc.file else {
                return out;
            };
            // Container label: the recipe header name when the file has
            // one, the file path otherwise so the editor still groups
            // declarations under something meaningful.
            let container = r.recipe_name().map(|s| s.to_string()).unwrap_or_else(|| {
                doc.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            #[allow(deprecated)]
            for ty in &r.types {
                out.push(SymbolInformation {
                    name: ty.name.clone(),
                    kind: SymbolKind::STRUCT,
                    location: Location {
                        uri: uri.clone(),
                        range: lsp_range(&doc.line_map, ty.span.clone()),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(container.clone()),
                });
            }
            #[allow(deprecated)]
            for en in &r.enums {
                out.push(SymbolInformation {
                    name: en.name.clone(),
                    kind: SymbolKind::ENUM,
                    location: Location {
                        uri: uri.clone(),
                        range: lsp_range(&doc.line_map, en.span.clone()),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(container.clone()),
                });
            }
            #[allow(deprecated)]
            for inp in &r.inputs {
                out.push(SymbolInformation {
                    name: format!("input {}", inp.name),
                    kind: SymbolKind::VARIABLE,
                    location: Location {
                        uri: uri.clone(),
                        range: lsp_range(&doc.line_map, inp.span.clone()),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: Some(container.clone()),
                });
            }
            // Steps too — they're top-level locatable nodes now.
            #[allow(deprecated)]
            for s in r.body.statements() {
                if let forage_core::ast::Statement::Step(step) = s {
                    out.push(SymbolInformation {
                        name: format!("step {}", step.name),
                        kind: SymbolKind::FUNCTION,
                        location: Location {
                            uri: uri.clone(),
                            range: lsp_range(&doc.line_map, step.span.clone()),
                        },
                        tags: None,
                        deprecated: None,
                        container_name: Some(container.clone()),
                    });
                }
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
        FieldType::Ref(n) => format!("Ref<{n}>"),
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
