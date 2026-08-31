#![allow(deprecated)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentLink, DocumentLinkOptions, DocumentLinkParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, InitializeParams, Location,
    MarkupContent, MarkupKind, OneOf, Position, PositionEncodingKind, PublishDiagnosticsParams,
    Range, ReferenceParams, ServerCapabilities, SymbolInformation, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceSymbolParams,
    WorkspaceSymbolResponse,
};
use maki_core::analysis::{
    AnalysisBlockKind, AnalysisDiagnosticKind, DateOrigin, DefinitionTarget, DefinitionTargetKind,
    DocumentAnalysis, HeadingOccurrence, LinkResolution, ProjectAnalysis, ReferenceDefinitionId,
    SourceSnapshot, analyze_project, property_description,
};
use maki_core::link_target::{DocumentSelector, InnerSelector, NoteLinkTarget};
use maki_core::source::{SourceMap, SourceSpan, Utf16Position};
use maki_core::{Maki, MakiConfig, is_discoverable_maki_path, list_maki_files};

pub type LspResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn run_stdio() -> LspResult<()> {
    run_stdio_with_version(env!("CARGO_PKG_VERSION"))
}

pub fn run_stdio_with_version(version: &str) -> LspResult<()> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialize) = connection.initialize_start()?;
    connection.initialize_finish(initialize_id, initialize_result(version))?;
    let params: InitializeParams = serde_json::from_value(initialize)?;
    let root = workspace_root(&params)?;
    let mut server = Server::new(root)?;

    server.run(&connection)?;
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn initialize_result(version: &str) -> serde_json::Value {
    serde_json::json!({
        "capabilities": server_capabilities(),
        "serverInfo": {
            "name": "maki",
            "version": version,
        },
    })
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(PositionEncodingKind::UTF16),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_link_provider: Some(DocumentLinkOptions {
            resolve_provider: Some(false),
            work_done_progress_options: Default::default(),
        }),
        completion_provider: Some(CompletionOptions::default()),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    }
}

fn workspace_root(params: &InitializeParams) -> LspResult<PathBuf> {
    let uri = params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first().map(|folder| folder.uri.clone()))
        .or_else(|| params.root_uri.clone())
        .ok_or("Maki LSP requires a workspace root")?;

    uri.to_file_path()
        .map_err(|_| format!("workspace URI is not a file URI: {uri}").into())
}

struct Server {
    source_root: PathBuf,
    documents: BTreeMap<PathBuf, String>,
    open_documents: BTreeSet<PathBuf>,
    analysis: ProjectAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReferenceIdentity {
    Document(PathBuf),
    Heading { path: PathBuf, anchor: String },
    Id { path: PathBuf, id: String },
}

impl Server {
    fn new(workspace_root: PathBuf) -> LspResult<Self> {
        let source_root = source_root(&workspace_root)?;
        let documents = load_documents(&source_root)?;
        let analysis = analyze_documents(&documents);

        Ok(Self {
            source_root,
            documents,
            open_documents: BTreeSet::new(),
            analysis,
        })
    }

    fn run(&mut self, connection: &Connection) -> LspResult<()> {
        for message in &connection.receiver {
            match message {
                Message::Request(request) => {
                    if connection.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    self.handle_request(connection, request)?;
                }
                Message::Notification(notification) => {
                    self.handle_notification(connection, notification)?;
                }
                Message::Response(_) => {}
            }
        }
        Ok(())
    }

    fn handle_request(&self, connection: &Connection, request: Request) -> LspResult<()> {
        let result = match request.method.as_str() {
            "textDocument/definition" => {
                let params: GotoDefinitionParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.definition(params))?
            }
            "textDocument/references" => {
                let params: ReferenceParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.references(params))?
            }
            "textDocument/documentLink" => {
                let params: DocumentLinkParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.document_links(params))?
            }
            "textDocument/completion" => {
                let params: CompletionParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.completion(params))?
            }
            "textDocument/documentSymbol" => {
                let params: DocumentSymbolParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.document_symbols(params))?
            }
            "workspace/symbol" => {
                let params: WorkspaceSymbolParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.workspace_symbols(params))?
            }
            "textDocument/hover" => {
                let params: HoverParams = serde_json::from_value(request.params)?;
                serde_json::to_value(self.hover(params))?
            }
            _ => {
                let response = Response::new_err(
                    request.id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("unsupported request: {}", request.method),
                );
                connection.sender.send(Message::Response(response))?;
                return Ok(());
            }
        };

        connection
            .sender
            .send(Message::Response(Response::new_ok(request.id, result)))?;
        Ok(())
    }

    fn handle_notification(
        &mut self,
        connection: &Connection,
        notification: Notification,
    ) -> LspResult<()> {
        match notification.method.as_str() {
            "textDocument/didOpen" => {
                let params: DidOpenTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                if let Some(path) = self.relative_path(&params.text_document.uri) {
                    self.documents
                        .insert(path.clone(), params.text_document.text);
                    self.open_documents.insert(path);
                    self.reanalyze();
                    self.publish_diagnostics(connection)?;
                }
            }
            "textDocument/didChange" => {
                let params: DidChangeTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                if let (Some(path), Some(change)) = (
                    self.relative_path(&params.text_document.uri),
                    params.content_changes.into_iter().last(),
                ) && self.open_documents.contains(&path)
                {
                    self.documents.insert(path, change.text);
                    self.reanalyze();
                    self.publish_diagnostics(connection)?;
                }
            }
            "textDocument/didClose" => {
                let params: DidCloseTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                if let Some(path) = self.relative_path(&params.text_document.uri) {
                    let absolute = self.source_root.join(&path);
                    match std::fs::read_to_string(absolute) {
                        Ok(source) => {
                            self.documents.insert(path.clone(), source);
                        }
                        Err(_) => {
                            self.documents.remove(&path);
                        }
                    }
                    self.open_documents.remove(&path);
                    self.reanalyze();
                    self.publish_empty_diagnostics(connection, &path)?;
                    self.publish_diagnostics(connection)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn reanalyze(&mut self) {
        self.analysis = analyze_documents(&self.documents);
    }

    fn relative_path(&self, uri: &Url) -> Option<PathBuf> {
        let relative = uri
            .to_file_path()
            .ok()?
            .strip_prefix(&self.source_root)
            .ok()
            .map(Path::to_path_buf)?;

        is_discoverable_maki_path(&relative).then_some(relative)
    }

    fn document_for_uri(&self, uri: &Url) -> Option<(&str, &DocumentAnalysis)> {
        let path = self.relative_path(uri)?;
        let source = self.documents.get(&path)?;
        let analysis = self.analysis.document(&path)?;
        Some((source, analysis))
    }

    fn publish_diagnostics(&self, connection: &Connection) -> LspResult<()> {
        for path in &self.open_documents {
            let Some(source) = self.documents.get(path) else {
                continue;
            };
            let diagnostics = self
                .analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.path == *path)
                .filter_map(|diagnostic| {
                    Some(Diagnostic {
                        range: lsp_range(source, diagnostic.span)?,
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("maki".to_string()),
                        code: Some(lsp_types::NumberOrString::String(
                            diagnostic_code(diagnostic.kind).to_string(),
                        )),
                        message: diagnostic.message.clone(),
                        ..Diagnostic::default()
                    })
                })
                .collect();
            self.send_diagnostics(connection, path, diagnostics)?;
        }
        Ok(())
    }

    fn publish_empty_diagnostics(&self, connection: &Connection, path: &Path) -> LspResult<()> {
        self.send_diagnostics(connection, path, Vec::new())
    }

    fn send_diagnostics(
        &self,
        connection: &Connection,
        path: &Path,
        diagnostics: Vec<Diagnostic>,
    ) -> LspResult<()> {
        let uri = Url::from_file_path(self.source_root.join(path))
            .map_err(|_| "failed to create document URI")?;
        let params = PublishDiagnosticsParams::new(uri, diagnostics, None);
        connection
            .sender
            .send(Message::Notification(Notification::new(
                "textDocument/publishDiagnostics".to_string(),
                params,
            )))?;
        Ok(())
    }

    fn definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let (source, document) = self.document_for_uri(uri)?;
        let offset = lsp_offset(source, params.text_document_position_params.position)?;

        if let Some(definition_id) = reference_use_definition_id_at(document, offset) {
            let definition = document.reference_graph.definition(definition_id)?;
            return Some(GotoDefinitionResponse::Scalar(
                self.symbol_location(&document.path, definition.key_span)?,
            ));
        }

        if let Some(occurrence) = document
            .note_links
            .iter()
            .find(|occurrence| span_touches(occurrence.span, offset))
        {
            let LinkResolution::Found(target) = occurrence.resolution.as_ref()? else {
                return None;
            };
            return Some(GotoDefinitionResponse::Scalar(self.location(target)?));
        }

        let definition_id = reference_declaration_definition_id_at(document, offset)?;
        let definition = document.reference_graph.definition(definition_id)?;
        Some(GotoDefinitionResponse::Scalar(
            self.symbol_location(&document.path, definition.key_span)?,
        ))
    }

    fn location(&self, target: &DefinitionTarget) -> Option<Location> {
        self.symbol_location(&target.path, target.selection_span)
    }

    fn references(&self, params: ReferenceParams) -> Option<Vec<Location>> {
        let uri = &params.text_document_position.text_document.uri;
        let path = self.relative_path(uri)?;
        let source = self.documents.get(&path)?;
        let document = self.analysis.document(&path)?;
        let offset = lsp_offset(source, params.text_document_position.position)?;

        if let Some(definition_id) = reference_use_definition_id_at(document, offset) {
            return Some(self.document_reference_locations(
                document,
                definition_id,
                params.context.include_declaration,
            ));
        }

        if document
            .note_links
            .iter()
            .any(|link| span_touches(link.span, offset))
        {
            let identity = self.reference_identity_at(document, offset)?;
            return Some(self.reference_locations(&identity, params.context.include_declaration));
        }

        if let Some(identity) = self.reference_identity_at(document, offset) {
            return Some(self.reference_locations(&identity, params.context.include_declaration));
        }

        let definition_id = reference_declaration_definition_id_at(document, offset)?;
        Some(self.document_reference_locations(
            document,
            definition_id,
            params.context.include_declaration,
        ))
    }

    fn document_reference_locations(
        &self,
        document: &DocumentAnalysis,
        definition_id: ReferenceDefinitionId,
        include_declaration: bool,
    ) -> Vec<Location> {
        let mut locations = Vec::new();
        if include_declaration
            && let Some(definition) = document.reference_graph.definition(definition_id)
            && let Some(location) = self.symbol_location(&document.path, definition.key_span)
        {
            locations.push(location);
        }

        locations.extend(
            document
                .reference_graph
                .uses_for(definition_id)
                .filter_map(|reference| self.symbol_location(&document.path, reference.key_span)),
        );

        locations
    }

    fn reference_identity_at(
        &self,
        document: &DocumentAnalysis,
        offset: usize,
    ) -> Option<ReferenceIdentity> {
        if let Some(link) = document
            .note_links
            .iter()
            .find(|link| span_touches(link.span, offset))
        {
            let LinkResolution::Found(target) = link.resolution.as_ref()? else {
                return None;
            };
            return self.reference_identity_for_target(target);
        }

        if let Some(block_id) = document
            .block_ids
            .iter()
            .find(|block_id| span_touches(block_id.value_span, offset))
        {
            return Some(ReferenceIdentity::Id {
                path: document.path.clone(),
                id: block_id.id.clone(),
            });
        }

        if let Some(heading) = document
            .headings
            .iter()
            .find(|heading| span_touches(heading.title_span, offset))
        {
            return Some(self.heading_reference_identity(document, heading));
        }

        if document.document_span.start < document.document_span.end
            && span_touches(document.document_span, offset)
        {
            return Some(ReferenceIdentity::Document(document.path.clone()));
        }

        None
    }

    fn reference_identity_for_target(
        &self,
        target: &DefinitionTarget,
    ) -> Option<ReferenceIdentity> {
        match target.kind {
            DefinitionTargetKind::Document => {
                Some(ReferenceIdentity::Document(target.path.clone()))
            }
            DefinitionTargetKind::Heading => {
                let anchor = target.fragment.as_ref()?;
                let document = self.analysis.document(&target.path)?;
                let heading = document
                    .headings
                    .iter()
                    .find(|heading| heading.title_span == target.selection_span)?;
                debug_assert_eq!(&heading.anchor, anchor);
                Some(self.heading_reference_identity(document, heading))
            }
            DefinitionTargetKind::Id => Some(ReferenceIdentity::Id {
                path: target.path.clone(),
                id: target.fragment.clone()?,
            }),
        }
    }

    fn heading_reference_identity(
        &self,
        document: &DocumentAnalysis,
        heading: &HeadingOccurrence,
    ) -> ReferenceIdentity {
        if document.block_ids.iter().any(|block_id| {
            block_id.owner_kind == AnalysisBlockKind::Heading
                && block_id.owner_span == heading.span
                && block_id.id == heading.anchor
        }) {
            ReferenceIdentity::Id {
                path: document.path.clone(),
                id: heading.anchor.clone(),
            }
        } else {
            ReferenceIdentity::Heading {
                path: document.path.clone(),
                anchor: heading.anchor.clone(),
            }
        }
    }

    fn reference_locations(
        &self,
        identity: &ReferenceIdentity,
        include_declaration: bool,
    ) -> Vec<Location> {
        let mut locations = Vec::new();
        if include_declaration {
            locations.extend(self.declaration_locations(identity));
        }

        for document in self.analysis.documents.values() {
            for link in &document.note_links {
                let Some(LinkResolution::Found(target)) = link.resolution.as_ref() else {
                    continue;
                };
                if self.reference_identity_for_target(target).as_ref() == Some(identity)
                    && let Some(location) = self.symbol_location(&document.path, link.target_span)
                {
                    locations.push(location);
                }
            }
        }
        locations
    }

    fn declaration_locations(&self, identity: &ReferenceIdentity) -> Vec<Location> {
        match identity {
            ReferenceIdentity::Document(path) => self
                .analysis
                .document(path)
                .and_then(|document| {
                    (document.document_span.start < document.document_span.end)
                        .then_some(document.document_span)
                })
                .and_then(|span| self.symbol_location(path, span))
                .into_iter()
                .collect(),
            ReferenceIdentity::Heading { path, anchor } => self
                .analysis
                .document(path)
                .into_iter()
                .flat_map(|document| &document.headings)
                .filter(|heading| heading.anchor == *anchor)
                .filter_map(|heading| self.symbol_location(path, heading.title_span))
                .collect(),
            ReferenceIdentity::Id { path, id } => self
                .analysis
                .document(path)
                .into_iter()
                .flat_map(|document| &document.block_ids)
                .filter(|block_id| block_id.id == *id)
                .filter_map(|block_id| self.symbol_location(path, block_id.value_span))
                .collect(),
        }
    }

    fn document_links(&self, params: DocumentLinkParams) -> Option<Vec<DocumentLink>> {
        let (source, document) = self.document_for_uri(&params.text_document.uri)?;
        Some(
            document
                .reference_links
                .iter()
                .filter_map(|reference| {
                    let target = Url::parse(&reference.target).ok()?;
                    matches!(target.scheme(), "http" | "https").then_some(DocumentLink {
                        range: lsp_range(source, reference.span)?,
                        target: Some(target),
                        tooltip: None,
                        data: None,
                    })
                })
                .collect(),
        )
    }

    fn completion(&self, params: CompletionParams) -> Option<CompletionResponse> {
        let uri = &params.text_document_position.text_document.uri;
        let path = self.relative_path(uri)?;
        let source = self.documents.get(&path)?;
        self.analysis.document(&path)?;
        let offset = lsp_offset(source, params.text_document_position.position)?;
        Some(CompletionResponse::Array(completion_items(
            &self.analysis,
            &path,
            source,
            offset,
        )))
    }

    fn document_symbols(&self, params: DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let (source, document) = self.document_for_uri(&params.text_document.uri)?;
        Some(DocumentSymbolResponse::Nested(heading_symbols(
            source,
            &document.headings,
        )))
    }

    fn workspace_symbols(&self, params: WorkspaceSymbolParams) -> WorkspaceSymbolResponse {
        let query = params.query.to_lowercase();
        let mut symbols = Vec::new();

        for document in self.analysis.documents.values() {
            if (query.is_empty() || document.title.to_lowercase().contains(&query))
                && let Some(location) = self.symbol_location(&document.path, document.document_span)
            {
                symbols.push(SymbolInformation {
                    name: document.title.clone(),
                    kind: SymbolKind::FILE,
                    tags: None,
                    deprecated: None,
                    location,
                    container_name: None,
                });
            }
            for heading in &document.headings {
                if (query.is_empty() || heading.title.to_lowercase().contains(&query))
                    && let Some(location) = self.symbol_location(&document.path, heading.title_span)
                {
                    symbols.push(SymbolInformation {
                        name: heading.title.clone(),
                        kind: SymbolKind::NAMESPACE,
                        tags: None,
                        deprecated: None,
                        location,
                        container_name: Some(document.title.clone()),
                    });
                }
            }
            for block_id in &document.block_ids {
                if (query.is_empty() || block_id.id.to_lowercase().contains(&query))
                    && let Some(location) =
                        self.symbol_location(&document.path, block_id.value_span)
                {
                    symbols.push(SymbolInformation {
                        name: block_id.id.clone(),
                        kind: SymbolKind::KEY,
                        tags: None,
                        deprecated: None,
                        location,
                        container_name: Some(document.title.clone()),
                    });
                }
            }
        }
        WorkspaceSymbolResponse::Flat(symbols)
    }

    fn symbol_location(&self, path: &Path, span: SourceSpan) -> Option<Location> {
        let source = self.documents.get(path)?;
        let uri = Url::from_file_path(self.source_root.join(path)).ok()?;
        Some(Location::new(uri, lsp_range(source, span)?))
    }

    fn hover(&self, params: HoverParams) -> Option<Hover> {
        let uri = &params.text_document_position_params.text_document.uri;
        let (source, document) = self.document_for_uri(uri)?;
        let offset = lsp_offset(source, params.text_document_position_params.position)?;

        if let Some(link) = document
            .note_links
            .iter()
            .find(|link| span_touches(link.span, offset))
        {
            return hover_markdown(link_hover(link), lsp_range(source, link.span));
        }
        if let Some(property) = document
            .properties
            .iter()
            .find(|property| span_touches(property.span, offset))
            && let Some(description) = property_description(&property.key)
        {
            return hover_markdown(
                format!("`{}`: {description}", property.key),
                lsp_range(source, property.span),
            );
        }
        if let Some(date) = document
            .dates
            .iter()
            .find(|date| span_touches(date.span, offset))
        {
            let origin = match &date.origin {
                DateOrigin::VisibleInline => "visible inline",
                DateOrigin::PropertyValue { key } => {
                    return hover_markdown(
                        format!("Maki date: `{}` (property:{key})", date.body),
                        lsp_range(source, date.span),
                    );
                }
            };
            return hover_markdown(
                format!("Maki date: `{}` ({origin})", date.body),
                lsp_range(source, date.span),
            );
        }
        None
    }
}

fn reference_use_definition_id_at(
    document: &DocumentAnalysis,
    offset: usize,
) -> Option<ReferenceDefinitionId> {
    document
        .reference_graph
        .uses
        .iter()
        .find(|reference| span_touches(reference.span, offset))
        .and_then(|reference| reference.definition_id)
}

fn reference_declaration_definition_id_at(
    document: &DocumentAnalysis,
    offset: usize,
) -> Option<ReferenceDefinitionId> {
    document
        .reference_graph
        .definitions
        .iter()
        .find(|definition| span_touches(definition.definition_span, offset))
        .map(|definition| definition.state.winner(definition.id))
}

fn source_root(workspace_root: &Path) -> LspResult<PathBuf> {
    let project_root = Maki::find_project_root(workspace_root)
        .map_err(|error| format!("failed to find Maki project: {error}"))?;
    if let Some(project_root) = project_root {
        let config = MakiConfig::load_project(&project_root)
            .map_err(|error| format!("failed to load Maki project: {error}"))?;
        return Ok(config.project_source_root(&project_root));
    }
    Ok(workspace_root.to_path_buf())
}

fn load_documents(root: &Path) -> LspResult<BTreeMap<PathBuf, String>> {
    let mut documents = BTreeMap::new();
    let files = list_maki_files(root)
        .map_err(|error| format!("failed to discover Maki documents: {error}"))?;
    for relative in files {
        let source = std::fs::read_to_string(root.join(&relative))?;
        documents.insert(relative, source);
    }
    Ok(documents)
}

fn analyze_documents(documents: &BTreeMap<PathBuf, String>) -> ProjectAnalysis {
    let snapshots = documents
        .iter()
        .map(|(path, source)| SourceSnapshot {
            path: path.as_path(),
            source,
        })
        .collect::<Vec<_>>();
    analyze_project(&snapshots)
}

fn completion_items(
    project: &ProjectAnalysis,
    current_path: &Path,
    source: &str,
    offset: usize,
) -> Vec<CompletionItem> {
    let map = SourceMap::new(source);
    let position = match map.position(offset) {
        Some(position) => position,
        None => return Vec::new(),
    };
    let line_span = match map.line_span(position.line) {
        Some(span) => span,
        None => return Vec::new(),
    };
    let before_cursor = &source[line_span.start..offset];
    let trimmed = before_cursor.trim_start_matches(' ');

    if let Some(open) = before_cursor.rfind("[[")
        && before_cursor[open + 2..].rfind("]]").is_none()
    {
        let target_start = line_span.start + open + 2;
        let target = &source[target_start..offset];
        return note_link_completion_items(
            project,
            current_path,
            source,
            target,
            target_start,
            offset,
        );
    }

    let property_body = trimmed
        .strip_prefix("--^ ")
        .or_else(|| trimmed.strip_prefix("--v "));
    if let Some(body) = property_body {
        if let Some((key, _)) = body.split_once(':') {
            let values: &[&str] = match key.trim().to_ascii_lowercase().as_str() {
                "mode" => &["block", "pre", "text"],
                "status" => &["todo", "doing", "done"],
                "lang" => &["maki", "rust", "javascript", "typescript", "nix", "shell"],
                _ => &[],
            };
            return values
                .iter()
                .map(|value| CompletionItem::new_simple((*value).to_string(), "value".to_string()))
                .collect();
        }
        return project
            .property_keys()
            .into_iter()
            .map(|key| CompletionItem {
                detail: property_description(&key).map(str::to_string),
                label: key,
                ..CompletionItem::default()
            })
            .collect();
    }

    Vec::new()
}

fn note_link_completion_items(
    project: &ProjectAnalysis,
    current_path: &Path,
    source: &str,
    target: &str,
    target_start: usize,
    offset: usize,
) -> Vec<CompletionItem> {
    let parsed = NoteLinkTarget::parse(target);
    let (prefix, replace_start) = match parsed.inner {
        Some(inner) => {
            let prefix = inner.target();
            (
                prefix,
                target_start + target.len().saturating_sub(prefix.len()),
            )
        }
        None => (parsed.document.target().unwrap_or_default(), target_start),
    };
    let Some(replace_range) = lsp_range(source, SourceSpan::new(replace_start, offset)) else {
        return Vec::new();
    };

    match parsed.inner {
        Some(InnerSelector::Heading(prefix)) => {
            let Some(document) = selected_document(project, current_path, parsed.document) else {
                return Vec::new();
            };
            document
                .headings
                .iter()
                .filter(|heading| prefix_matches(&heading.anchor, prefix))
                .map(|heading| {
                    completion_item(
                        heading.anchor.clone(),
                        format!("Maki heading in {}", document.title),
                        replace_range,
                    )
                })
                .collect()
        }
        Some(InnerSelector::Id(prefix)) => {
            let Some(document) = selected_document(project, current_path, parsed.document) else {
                return Vec::new();
            };
            let mut seen = BTreeSet::new();
            document
                .block_ids
                .iter()
                .filter(|block_id| block_id.id.starts_with(prefix))
                .filter(|block_id| seen.insert(block_id.id.clone()))
                .map(|block_id| {
                    completion_item(
                        block_id.id.clone(),
                        format!("Maki explicit ID in {}", document.title),
                        replace_range,
                    )
                })
                .collect()
        }
        None => document_completion_items(
            project,
            current_path,
            parsed.document,
            prefix,
            replace_range,
        ),
    }
}

fn completion_item(label: String, detail: String, range: Range) -> CompletionItem {
    CompletionItem {
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: label.clone(),
        })),
        label,
        detail: Some(detail),
        ..CompletionItem::default()
    }
}

fn document_completion_items(
    project: &ProjectAnalysis,
    current_path: &Path,
    selector: DocumentSelector<'_>,
    prefix: &str,
    replace_range: Range,
) -> Vec<CompletionItem> {
    match selector {
        DocumentSelector::Root(_) => project
            .note_candidates()
            .filter(|document| prefix_matches(&document.canonical_path, prefix))
            .map(|document| {
                completion_item(
                    format!("/{}", document.canonical_path),
                    document.title.clone(),
                    replace_range,
                )
            })
            .collect(),
        DocumentSelector::Child(_) => {
            let Some(current) = project.document(current_path) else {
                return Vec::new();
            };
            let child_root = format!("{}/", current.canonical_path);
            let mut items = project
                .note_candidates()
                .filter_map(|document| {
                    let relative = document.canonical_path.strip_prefix(&child_root)?;
                    prefix_matches(relative, prefix).then_some((document, relative))
                })
                .map(|(document, relative)| {
                    completion_item(
                        format!("+{relative}"),
                        document.title.clone(),
                        replace_range,
                    )
                })
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                left.label
                    .matches('/')
                    .count()
                    .cmp(&right.label.matches('/').count())
                    .then_with(|| left.label.cmp(&right.label))
            });
            items
        }
        DocumentSelector::Legacy(_) => project
            .note_candidates()
            .filter(|document| {
                prefix_matches(&document.canonical_path, prefix)
                    || prefix_matches(&document.title, prefix)
            })
            .map(|document| {
                completion_item(
                    document.canonical_path.clone(),
                    document.title.clone(),
                    replace_range,
                )
            })
            .collect(),
        DocumentSelector::Current => Vec::new(),
    }
}

fn selected_document<'a>(
    project: &'a ProjectAnalysis,
    current_path: &Path,
    selector: DocumentSelector<'_>,
) -> Option<&'a DocumentAnalysis> {
    match selector {
        DocumentSelector::Current => project.document(current_path),
        DocumentSelector::Root(target) => coordinate_document(project, target),
        DocumentSelector::Child(target) => {
            let target = normalize_document_target(target);
            if !is_normal_relative_target(target) {
                return None;
            }
            let current = project.document(current_path)?;
            coordinate_document(project, &format!("{}/{target}", current.canonical_path))
        }
        DocumentSelector::Legacy(target) => legacy_document(project, current_path, target),
    }
}

fn coordinate_document<'a>(
    project: &'a ProjectAnalysis,
    target: &str,
) -> Option<&'a DocumentAnalysis> {
    let target = normalize_document_target(target);
    project
        .note_candidates()
        .find(|document| document.canonical_path == target)
        .or_else(|| {
            let normalized = target.to_lowercase();
            unique_document(
                project
                    .note_candidates()
                    .filter(|document| document.canonical_path.to_lowercase() == normalized),
            )
        })
}

fn legacy_document<'a>(
    project: &'a ProjectAnalysis,
    current_path: &Path,
    target: &str,
) -> Option<&'a DocumentAnalysis> {
    let target = normalize_document_target(target);
    if target.is_empty() {
        return None;
    }
    if let Some(exact) = project
        .note_candidates()
        .find(|document| document.canonical_path == target)
    {
        return Some(exact);
    }
    if target.contains('/') {
        return coordinate_document(project, target);
    }

    let sibling = current_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{target}.maki"));
    let sibling = canonical_source_path(&sibling);
    if let Some(document) = coordinate_document(project, &sibling) {
        return Some(document);
    }

    let normalized = target.to_lowercase();
    unique_document(project.note_candidates().filter(|document| {
        document
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.to_lowercase() == normalized)
    }))
}

fn unique_document<'a>(
    mut candidates: impl Iterator<Item = &'a DocumentAnalysis>,
) -> Option<&'a DocumentAnalysis> {
    let candidate = candidates.next()?;
    candidates.next().is_none().then_some(candidate)
}

fn normalize_document_target(target: &str) -> &str {
    target.strip_suffix(".maki").unwrap_or(target)
}

fn prefix_matches(value: &str, prefix: &str) -> bool {
    value.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn canonical_source_path(path: &Path) -> String {
    let path = path.with_extension("");
    path.strip_prefix(".")
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned()
}

fn is_normal_relative_target(target: &str) -> bool {
    !target.is_empty()
        && target
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn heading_symbols(source: &str, headings: &[HeadingOccurrence]) -> Vec<DocumentSymbol> {
    fn section_span(
        source_len: usize,
        headings: &[HeadingOccurrence],
        heading_index: usize,
    ) -> SourceSpan {
        let heading = &headings[heading_index];
        let end = headings[heading_index + 1..]
            .iter()
            .find(|candidate| candidate.level <= heading.level)
            .map_or(source_len, |candidate| candidate.span.start);
        SourceSpan::new(heading.span.start, end)
    }

    fn build(
        source: &str,
        headings: &[HeadingOccurrence],
        index: &mut usize,
        parent_level: usize,
    ) -> Vec<DocumentSymbol> {
        let mut symbols = Vec::new();
        while let Some(heading) = headings.get(*index) {
            if heading.level <= parent_level {
                break;
            }
            let heading_index = *index;
            *index += 1;
            let children = build(source, headings, index, heading.level);
            let section = section_span(source.len(), headings, heading_index);
            let symbol_span = SourceSpan::new(heading.title_span.start, section.end);
            let (Some(range), Some(selection_range)) = (
                lsp_range(source, symbol_span),
                lsp_range(source, heading.title_span),
            ) else {
                continue;
            };
            symbols.push(DocumentSymbol {
                name: heading.title.clone(),
                detail: (heading.anchor != heading.title).then(|| format!("#{}", heading.anchor)),
                kind: SymbolKind::NAMESPACE,
                tags: None,
                deprecated: None,
                range,
                selection_range,
                children: (!children.is_empty()).then_some(children),
            });
        }
        symbols
    }

    build(source, headings, &mut 0, 0)
}

fn link_hover(link: &maki_core::analysis::NoteLinkOccurrence) -> String {
    match link.resolution.as_ref() {
        Some(LinkResolution::Found(target)) => {
            let selector = match target.kind {
                DefinitionTargetKind::Document => String::new(),
                DefinitionTargetKind::Heading => target
                    .fragment
                    .as_deref()
                    .map_or(String::new(), |anchor| format!("#{anchor}")),
                DefinitionTargetKind::Id => target
                    .fragment
                    .as_deref()
                    .map_or(String::new(), |id| format!("@{id}")),
            };
            format!(
                "Resolves to `/{document}{selector}`.",
                document = canonical_source_path(&target.path)
            )
        }
        Some(LinkResolution::BrokenNote) => "Target note was not found.".to_string(),
        Some(LinkResolution::AmbiguousNote) => "Target note is ambiguous.".to_string(),
        Some(LinkResolution::BrokenHeading) => "Target heading was not found.".to_string(),
        Some(LinkResolution::AmbiguousHeading) => "Target heading is ambiguous.".to_string(),
        Some(LinkResolution::BrokenId) => "Target explicit ID was not found.".to_string(),
        Some(LinkResolution::AmbiguousId) => "Target explicit ID is ambiguous.".to_string(),
        None => "Link has not been resolved.".to_string(),
    }
}

fn hover_markdown(value: String, range: Option<Range>) -> Option<Hover> {
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    })
}

fn lsp_range(source: &str, span: SourceSpan) -> Option<Range> {
    let map = SourceMap::new(source);
    Some(Range::new(
        lsp_position(map.utf16_position(span.start)?),
        lsp_position(map.utf16_position(span.end)?),
    ))
}

fn lsp_position(position: Utf16Position) -> Position {
    Position::new(position.line as u32, position.character as u32)
}

fn lsp_offset(source: &str, position: Position) -> Option<usize> {
    SourceMap::new(source).offset_utf16(Utf16Position {
        line: position.line as usize,
        character: position.character as usize,
    })
}

fn span_touches(span: SourceSpan, offset: usize) -> bool {
    span.contains(offset)
}

fn diagnostic_code(kind: AnalysisDiagnosticKind) -> &'static str {
    match kind {
        AnalysisDiagnosticKind::ParseWarning => "parse-warning",
        AnalysisDiagnosticKind::DuplicateId => "duplicate-id",
        AnalysisDiagnosticKind::UnresolvedReference => "unresolved-reference",
        AnalysisDiagnosticKind::BrokenNoteLink => "broken-note-link",
        AnalysisDiagnosticKind::AmbiguousNoteLink => "ambiguous-note-link",
        AnalysisDiagnosticKind::BrokenHeadingLink => "broken-heading-link",
        AnalysisDiagnosticKind::AmbiguousHeadingLink => "ambiguous-heading-link",
        AnalysisDiagnosticKind::BrokenIdLink => "broken-id-link",
        AnalysisDiagnosticKind::AmbiguousIdLink => "ambiguous-id-link",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestWorkspace {
        root: PathBuf,
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn temp_workspace(name: &str) -> TestWorkspace {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("maki-lsp-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        TestWorkspace { root }
    }

    fn write_workspace_file(workspace: &TestWorkspace, path: &str, content: &str) {
        let path = workspace.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn test_server(documents: BTreeMap<PathBuf, String>) -> Server {
        Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        }
    }

    fn document_uri(path: &str) -> Url {
        Url::parse(&format!("file:///workspace/{path}")).unwrap()
    }

    fn definition_at(server: &Server, path: &str, position: Position) -> Location {
        let response = server
            .definition(GotoDefinitionParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(document_uri(path)),
                    position,
                ),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .expect("position should resolve to a definition");
        let GotoDefinitionResponse::Scalar(location) = response else {
            panic!("definition should be a scalar location");
        };
        location
    }

    fn references_at(
        server: &Server,
        path: &str,
        position: Position,
        include_declaration: bool,
    ) -> Vec<Location> {
        server
            .references(ReferenceParams {
                text_document_position: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(document_uri(path)),
                    position,
                ),
                context: lsp_types::ReferenceContext {
                    include_declaration,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .expect("position should have references")
    }

    fn document_links_for(server: &Server, path: &str) -> Vec<DocumentLink> {
        server
            .document_links(DocumentLinkParams {
                text_document: lsp_types::TextDocumentIdentifier::new(document_uri(path)),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .expect("document should be analyzed")
    }

    #[test]
    fn initialize_result_reports_server_version() {
        let result = initialize_result("1.2.3");

        assert_eq!(result["serverInfo"]["name"], "maki");
        assert_eq!(result["serverInfo"]["version"], "1.2.3");
        assert!(result["capabilities"].is_object());
        assert_eq!(
            result["capabilities"]["documentLinkProvider"]["resolveProvider"],
            false
        );
        assert_eq!(result["capabilities"]["referencesProvider"], true);
    }

    #[test]
    fn workspace_loading_bounds_duplicated_hidden_and_generated_trees() {
        let workspace = temp_workspace("hidden-tree");
        write_workspace_file(&workspace, "index.maki", "= Included");
        write_workspace_file(&workspace, ".hidden.maki", "= Excluded");
        write_workspace_file(&workspace, ".git/README.maki", "= Excluded");
        write_workspace_file(&workspace, ".jj/repo/store.maki", "= Excluded");
        write_workspace_file(&workspace, "node_modules/package/README.maki", "= Excluded");
        write_workspace_file(&workspace, "target/generated.maki", "= Excluded");

        for copy in 0..8 {
            for note in 0..8 {
                write_workspace_file(
                    &workspace,
                    &format!(".direnv/flake-inputs/copy-{copy}/notes/{note}.maki"),
                    "= Duplicated hidden note",
                );
            }
        }

        let server = Server::new(workspace.root.clone()).unwrap();

        assert_eq!(
            server.documents.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("index.maki")]
        );
        assert_eq!(server.analysis.documents.len(), 1);

        let hidden_uri = Url::from_file_path(
            workspace
                .root
                .join(".direnv/flake-inputs/copy-0/notes/0.maki"),
        )
        .unwrap();
        assert_eq!(server.relative_path(&hidden_uri), None);

        let outside_uri =
            Url::from_file_path(workspace.root.parent().unwrap().join("outside.maki")).unwrap();
        assert_eq!(server.relative_path(&outside_uri), None);
    }

    #[test]
    fn lsp_ranges_use_utf16_columns() {
        let source = "한글 😀 link";
        let span = SourceSpan::new(source.find("link").unwrap(), source.len());

        assert_eq!(
            lsp_range(source, span),
            Some(Range::new(Position::new(0, 6), Position::new(0, 10)))
        );
    }

    #[test]
    fn completion_offers_notes_headings_and_property_keys() {
        let documents = BTreeMap::from([
            (PathBuf::from("index.maki"), "[[ot".to_string()),
            (PathBuf::from("other.maki"), "= Heading".to_string()),
        ]);
        let project = analyze_documents(&documents);
        let current_path = Path::new("index.maki");
        let notes = completion_items(&project, current_path, "[[ot", 4);
        let headings = completion_items(&project, current_path, "[[other#H", 9);
        let properties = completion_items(&project, current_path, "--v ti", 6);

        assert!(notes.iter().any(|item| item.label == "other"));
        assert!(headings.iter().any(|item| item.label == "Heading"));
        assert!(properties.iter().any(|item| item.label == "title"));
    }

    #[test]
    fn note_link_completion_respects_document_and_inner_selector_scope() {
        let current_source = "--^ title: Current\n\n= Current heading\n--^ id: current-heading\n\ncurrent body\n--^ id: current-only\n";
        let documents = BTreeMap::from([
            (
                PathBuf::from("plans/current.maki"),
                current_source.to_string(),
            ),
            (
                PathBuf::from("plans/current/child.maki"),
                "--^ title: Child\n\n= Child heading\n--^ id: child-heading\n\nchild body\n--^ id: child-only\n"
                    .to_string(),
            ),
            (
                PathBuf::from("plans/current/child/deep.maki"),
                "--^ title: Deep\n\ndeep".to_string(),
            ),
            (
                PathBuf::from("plans/cousin.maki"),
                "--^ title: Cousin\n\ncousin body\n--^ id: cousin-only\n".to_string(),
            ),
            (
                PathBuf::from("root.maki"),
                "--^ title: Root\n\n= Root heading\n\nroot body\n--^ id: root-only\n"
                    .to_string(),
            ),
            (
                PathBuf::from("unrelated.maki"),
                "unrelated body\n--^ id: unrelated-only\n".to_string(),
            ),
        ]);
        let project = analyze_documents(&documents);
        let current = Path::new("plans/current.maki");

        let labels = |source: &str| {
            completion_items(&project, current, source, source.len())
                .into_iter()
                .map(|item| item.label)
                .collect::<Vec<_>>()
        };

        assert_eq!(labels("[[#current-"), vec!["current-heading"]);
        assert_eq!(
            labels("[[@current-"),
            vec!["current-heading", "current-only"]
        );
        assert_eq!(labels("[[/ROOT#root"), vec!["Root heading"]);
        assert_eq!(labels("[[/root@root-"), vec!["root-only"]);
        assert_eq!(labels("[[+CHILD#CHILD-"), vec!["child-heading"]);
        assert_eq!(
            labels("[[+child@child-"),
            vec!["child-heading", "child-only"]
        );
        assert_eq!(labels("[[/RO"), vec!["/root"]);
        assert_eq!(labels("[[+c"), vec!["+child", "+child/deep"]);
        assert!(labels("[[@CURRENT-").is_empty());

        let root_items = completion_items(&project, current, "[[/ro", "[[/ro".len());
        let Some(CompletionTextEdit::Edit(edit)) = &root_items[0].text_edit else {
            panic!("root completion should use a text edit");
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 2), Position::new(0, 5))
        );
        assert_eq!(edit.new_text, "/root");

        let id_source = "[[/root@ro";
        let id_items = completion_items(&project, current, id_source, id_source.len());
        let Some(CompletionTextEdit::Edit(edit)) = &id_items[0].text_edit else {
            panic!("ID completion should use a text edit");
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 8), Position::new(0, 10))
        );
        assert_eq!(edit.new_text, "root-only");

        let child_items = completion_items(&project, current, "[[+c", "[[+c".len());
        let Some(CompletionTextEdit::Edit(edit)) = &child_items[0].text_edit else {
            panic!("child completion should use a text edit");
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 2), Position::new(0, 4))
        );
        assert_eq!(edit.new_text, "+child");

        let heading_source = "[[/ROOT#root";
        let heading_items =
            completion_items(&project, current, heading_source, heading_source.len());
        let Some(CompletionTextEdit::Edit(edit)) = &heading_items[0].text_edit else {
            panic!("qualified heading completion should use a text edit");
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 8), Position::new(0, 12))
        );
        assert_eq!(edit.new_text, "Root heading");

        let child_id_source = "[[+CHILD@child-";
        let child_id_items =
            completion_items(&project, current, child_id_source, child_id_source.len());
        let Some(CompletionTextEdit::Edit(edit)) = &child_id_items[0].text_edit else {
            panic!("qualified child ID completion should use a text edit");
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 9), Position::new(0, 15))
        );
        assert_eq!(edit.new_text, "child-heading");
    }

    #[test]
    fn completion_offers_current_quote_modes() {
        let project = analyze_documents(&BTreeMap::new());
        let source = "--v mode: ";
        let modes = completion_items(&project, Path::new("index.maki"), source, source.len())
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();

        assert_eq!(modes, vec!["block", "pre", "text"]);
    }

    #[test]
    fn document_links_open_url_reference_markers() {
        let source = r#"- [요카토 추천 리스트][]
  - [김치][]
- [로컬][]

[요카토 추천 리스트]: <https://docs.google.com/document/d/example/edit>
[김치]: <https://hakkeido.com/>
[로컬]: other
"#;
        let documents = BTreeMap::from([(PathBuf::from("index.maki"), source.to_string())]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        };
        let params = DocumentLinkParams {
            text_document: lsp_types::TextDocumentIdentifier {
                uri: Url::parse("file:///workspace/index.maki").unwrap(),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let links = server.document_links(params).unwrap();

        assert_eq!(links.len(), 2);
        assert_eq!(
            links[0].target.as_ref().map(Url::as_str),
            Some("https://docs.google.com/document/d/example/edit")
        );
        assert_eq!(
            links[1].target.as_ref().map(Url::as_str),
            Some("https://hakkeido.com/")
        );
        assert_eq!(
            links[1].range,
            Range::new(Position::new(1, 4), Position::new(1, 10))
        );
    }

    #[test]
    fn document_references_support_definition_and_both_presentations() {
        let source = "[topic][] [^topic][]\n\n[topic]: <https://example.com/topic>\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));
        let definition = Location::new(
            document_uri("index.maki"),
            Range::new(Position::new(2, 1), Position::new(2, 6)),
        );
        let uses = vec![
            Location::new(
                document_uri("index.maki"),
                Range::new(Position::new(0, 1), Position::new(0, 6)),
            ),
            Location::new(
                document_uri("index.maki"),
                Range::new(Position::new(0, 12), Position::new(0, 17)),
            ),
        ];

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 2)),
            definition
        );
        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 13)),
            definition
        );
        assert_eq!(
            definition_at(&server, "index.maki", Position::new(2, 0)),
            definition
        );
        assert_eq!(
            definition_at(&server, "index.maki", Position::new(2, 20)),
            definition
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 2), false),
            uses
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 13), true),
            std::iter::once(definition.clone())
                .chain(uses.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(2, 2), false),
            uses
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(2, 20), false),
            uses
        );
    }

    #[test]
    fn document_references_never_cross_document_boundaries() {
        let documents = BTreeMap::from([
            (
                PathBuf::from("a.maki"),
                "[shared][] [^shared][]\n\n[shared]: <https://a.example/>\n".to_string(),
            ),
            (
                PathBuf::from("b.maki"),
                "[shared][] [^shared][]\n\n[shared]: <https://b.example/>\n".to_string(),
            ),
        ]);
        let server = test_server(documents);

        let definition = definition_at(&server, "a.maki", Position::new(0, 2));
        assert_eq!(definition.uri, document_uri("a.maki"));
        let references = references_at(&server, "a.maki", Position::new(0, 2), true);
        assert_eq!(references.len(), 3);
        assert!(
            references
                .iter()
                .all(|location| location.uri == document_uri("a.maki"))
        );
    }

    #[test]
    fn duplicate_definition_navigation_uses_the_canonical_winner() {
        let source = "[dup][] [^dup][]\n\n[dup]: <https://first.example/>\n[dup]: <https://second.example/>\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));
        let winner = Location::new(
            document_uri("index.maki"),
            Range::new(Position::new(2, 1), Position::new(2, 4)),
        );

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(3, 3)),
            winner
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(3, 3), true),
            vec![
                winner,
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 1), Position::new(0, 4)),
                ),
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 10), Position::new(0, 13)),
                ),
            ]
        );
        let links = document_links_for(&server, "index.maki");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].target.as_ref().map(Url::as_str),
            Some("https://first.example/")
        );
    }

    #[test]
    fn document_links_open_only_link_presentation_markers() {
        let source =
            "[url][] [^url][] *[url][]* ::[^url][]::\n\n[url]: <https://example.com/path>\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));

        let links = document_links_for(&server, "index.maki");

        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|link| {
            link.target.as_ref().map(Url::as_str) == Some("https://example.com/path")
        }));
        assert_eq!(
            links.iter().map(|link| link.range).collect::<Vec<_>>(),
            vec![
                Range::new(Position::new(0, 0), Position::new(0, 7)),
                Range::new(Position::new(0, 18), Position::new(0, 25)),
            ]
        );
    }

    #[test]
    fn document_links_require_an_exact_hyperlink_value() {
        let source = "[url][] [prose][]\n\n[url]: <https://example.com/path>\n[prose]: <https://example.com> has details\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));

        let links = document_links_for(&server, "index.maki");

        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].target.as_ref().map(Url::as_str),
            Some("https://example.com/path")
        );
        assert_eq!(
            links[0].range,
            Range::new(Position::new(0, 0), Position::new(0, 7))
        );
    }

    #[test]
    fn document_links_include_direct_http_links_with_the_full_construct_range() {
        let source = "[site](https://example.com) [local](page)";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));

        let links = document_links_for(&server, "index.maki");

        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].target.as_ref().map(Url::as_str),
            Some("https://example.com/")
        );
        assert_eq!(
            links[0].range,
            Range::new(Position::new(0, 0), Position::new(0, 27))
        );
    }

    #[test]
    fn definition_line_navigation_prefers_nested_semantic_targets() {
        let documents = BTreeMap::from([
            (
                PathBuf::from("index.maki"),
                "[ref]: See [[target]]\n".to_string(),
            ),
            (
                PathBuf::from("target.maki"),
                "--^ title: Target\n\nbody\n".to_string(),
            ),
        ]);
        let server = test_server(documents);

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 14)),
            Location::new(
                document_uri("target.maki"),
                Range::new(Position::new(0, 11), Position::new(0, 17)),
            )
        );
        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 8)),
            Location::new(
                document_uri("index.maki"),
                Range::new(Position::new(0, 1), Position::new(0, 4)),
            )
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 14), true),
            vec![
                Location::new(
                    document_uri("target.maki"),
                    Range::new(Position::new(0, 11), Position::new(0, 17)),
                ),
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 13), Position::new(0, 19)),
                ),
            ]
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 8), true),
            vec![Location::new(
                document_uri("index.maki"),
                Range::new(Position::new(0, 1), Position::new(0, 4)),
            )]
        );
    }

    #[test]
    fn exact_note_link_reference_targets_keep_project_navigation() {
        let documents = BTreeMap::from([
            (
                PathBuf::from("index.maki"),
                "[ref][]\n\n[ref]: [[target]]\n".to_string(),
            ),
            (
                PathBuf::from("target.maki"),
                "--^ title: Target\n\nbody\n".to_string(),
            ),
        ]);
        let server = test_server(documents);

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(2, 10)),
            Location::new(
                document_uri("target.maki"),
                Range::new(Position::new(0, 11), Position::new(0, 17)),
            )
        );
    }

    #[test]
    fn date_reference_use_hover_reports_the_semantic_target_at_the_marker() {
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            "[deadline][]\n\n[deadline]: [2026-08-31]\n".to_string(),
        )]));

        let hover = server
            .hover(HoverParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(document_uri("index.maki")),
                    Position::new(0, 3),
                ),
                work_done_progress_params: Default::default(),
            })
            .unwrap();
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("date hover should use markup content");
        };
        assert_eq!(markup.value, "Maki date: `2026-08-31` (visible inline)");
        assert_eq!(
            hover.range,
            Some(Range::new(Position::new(0, 0), Position::new(0, 12)))
        );
    }

    #[test]
    fn adjacent_reference_markers_use_half_open_utf16_spans() {
        let source = "😀[a][b] [^a][b]\n\n[a]: https://a.example/\n[b]: https://b.example/\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));
        let b_definition = Location::new(
            document_uri("index.maki"),
            Range::new(Position::new(3, 1), Position::new(3, 2)),
        );

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 5)),
            b_definition
        );
        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 13)),
            b_definition
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 5), false),
            vec![
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 6), Position::new(0, 7)),
                ),
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 14), Position::new(0, 15)),
                ),
            ]
        );
    }

    #[test]
    fn document_reference_locations_use_utf16_code_units() {
        let source = "😀 [키😀][] [^키😀][]\n\n[키😀]: <https://emoji.example/>\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));
        let declaration = Location::new(
            document_uri("index.maki"),
            Range::new(Position::new(2, 1), Position::new(2, 4)),
        );

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(0, 11)),
            declaration
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(0, 4), true),
            vec![
                declaration,
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 4), Position::new(0, 7)),
                ),
                Location::new(
                    document_uri("index.maki"),
                    Range::new(Position::new(0, 13), Position::new(0, 16)),
                ),
            ]
        );
        let links = document_links_for(&server, "index.maki");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].range,
            Range::new(Position::new(0, 3), Position::new(0, 10))
        );
    }

    #[test]
    fn document_references_cover_list_table_and_nested_formatting() {
        let source = "- [ctx][]\n  - *[^ctx][]*\n| reference |\n|---|\n| ::[ctx][]:: [^ctx][] |\n\n[ctx]: <https://example.com/context>\n";
        let server = test_server(BTreeMap::from([(
            PathBuf::from("index.maki"),
            source.to_string(),
        )]));

        assert_eq!(
            definition_at(&server, "index.maki", Position::new(1, 8)).range,
            Range::new(Position::new(6, 1), Position::new(6, 4))
        );
        assert_eq!(
            references_at(&server, "index.maki", Position::new(4, 6), true)
                .into_iter()
                .map(|location| location.range)
                .collect::<Vec<_>>(),
            vec![
                Range::new(Position::new(6, 1), Position::new(6, 4)),
                Range::new(Position::new(0, 3), Position::new(0, 6)),
                Range::new(Position::new(1, 7), Position::new(1, 10)),
                Range::new(Position::new(4, 5), Position::new(4, 8)),
                Range::new(Position::new(4, 16), Position::new(4, 19)),
            ]
        );
        assert_eq!(
            document_links_for(&server, "index.maki")
                .into_iter()
                .map(|link| link.range)
                .collect::<Vec<_>>(),
            vec![
                Range::new(Position::new(0, 2), Position::new(0, 9)),
                Range::new(Position::new(4, 4), Position::new(4, 11)),
            ]
        );
    }

    #[test]
    fn id_definition_references_and_hover_use_document_local_identity() {
        let plans = "--^ title: Plans\n\nplans body\n--^ id: local-id\n\n= Stable heading\n--^ id: heading-id\n\n[[@local-id]]\n[[#heading-id]]\n[[@heading-id]]\n";
        let other = "--^ title: Other\n\nother body\n--^ id: local-id\n\n[[/plans@local-id]]\n[[/plans#heading-id]]\n[[/plans@heading-id]]\n[[@local-id]]\n";
        let documents = BTreeMap::from([
            (PathBuf::from("other.maki"), other.to_string()),
            (PathBuf::from("plans.maki"), plans.to_string()),
        ]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        };
        let plans_uri = Url::parse("file:///workspace/plans.maki").unwrap();

        let declaration_position = Position::new(3, 10);
        let references = server
            .references(ReferenceParams {
                text_document_position: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(plans_uri.clone()),
                    declaration_position,
                ),
                context: lsp_types::ReferenceContext {
                    include_declaration: true,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .unwrap();

        assert_eq!(references.len(), 3);
        assert_eq!(
            references
                .iter()
                .filter(|location| location.uri.as_str() == "file:///workspace/plans.maki")
                .count(),
            2
        );
        assert_eq!(
            references
                .iter()
                .filter(|location| location.uri.as_str() == "file:///workspace/other.maki")
                .count(),
            1
        );
        assert_eq!(
            references[0].range,
            Range::new(Position::new(3, 8), Position::new(3, 16))
        );

        let heading_references = server
            .references(ReferenceParams {
                text_document_position: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(plans_uri.clone()),
                    Position::new(9, 4),
                ),
                context: lsp_types::ReferenceContext {
                    include_declaration: false,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .unwrap();
        assert_eq!(heading_references.len(), 4);

        let definition = server
            .definition(GotoDefinitionParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(plans_uri.clone()),
                    Position::new(8, 4),
                ),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .unwrap();
        let GotoDefinitionResponse::Scalar(definition) = definition else {
            panic!("ID definition should be a scalar location");
        };
        assert_eq!(definition.uri, plans_uri);
        assert_eq!(
            definition.range,
            Range::new(Position::new(3, 8), Position::new(3, 16))
        );

        let hover = server
            .hover(HoverParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams::new(
                    lsp_types::TextDocumentIdentifier::new(plans_uri),
                    Position::new(8, 4),
                ),
                work_done_progress_params: Default::default(),
            })
            .unwrap();
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("link hover should use markup content");
        };
        assert_eq!(markup.value, "Resolves to `/plans@local-id`.");
    }

    #[test]
    fn reference_identity_prefers_nested_links_and_matches_the_specific_heading_owner() {
        let source = "= See [[/other]]\n\n= alpha\n\n= beta\n--^ id: alpha\n";
        let documents = BTreeMap::from([
            (PathBuf::from("current.maki"), source.to_string()),
            (PathBuf::from("other.maki"), "other".to_string()),
        ]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        };
        let document = server.analysis.document(Path::new("current.maki")).unwrap();

        assert_eq!(
            server.reference_identity_at(document, source.find("/other").unwrap() + 1),
            Some(ReferenceIdentity::Document(PathBuf::from("other.maki")))
        );
        assert_eq!(
            server.reference_identity_at(document, source.find("alpha").unwrap()),
            Some(ReferenceIdentity::Heading {
                path: PathBuf::from("current.maki"),
                anchor: "alpha".to_string(),
            })
        );
        assert_eq!(
            server.reference_identity_at(document, source.find("beta").unwrap()),
            Some(ReferenceIdentity::Id {
                path: PathBuf::from("current.maki"),
                id: "alpha".to_string(),
            })
        );
    }

    #[test]
    fn diagnostics_are_published_only_for_open_documents() {
        let documents = BTreeMap::from([
            (PathBuf::from("open.maki"), "[[missing-open]]".to_string()),
            (
                PathBuf::from("closed.maki"),
                "[[missing-closed]]".to_string(),
            ),
        ]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::from([PathBuf::from("open.maki")]),
            documents,
        };
        let (server_connection, client_connection) = Connection::memory();

        server.publish_diagnostics(&server_connection).unwrap();

        let messages = client_connection.receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        let Message::Notification(notification) = &messages[0] else {
            panic!("expected a diagnostics notification");
        };
        let params: PublishDiagnosticsParams =
            serde_json::from_value(notification.params.clone()).unwrap();
        assert_eq!(params.uri.as_str(), "file:///workspace/open.maki");
        assert_eq!(params.diagnostics.len(), 1);
        assert_eq!(
            params.diagnostics[0].code,
            Some(lsp_types::NumberOrString::String(
                "broken-note-link".to_string()
            ))
        );
    }

    #[test]
    fn diagnostics_publish_unresolved_reference_key_ranges_in_utf16() {
        let source = "😀 [ missing ][] [^][missing] [missing]\n";
        let documents = BTreeMap::from([(PathBuf::from("open.maki"), source.to_string())]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::from([PathBuf::from("open.maki")]),
            documents,
        };
        let (server_connection, client_connection) = Connection::memory();

        server.publish_diagnostics(&server_connection).unwrap();

        let Message::Notification(notification) = client_connection.receiver.recv().unwrap() else {
            panic!("expected a diagnostics notification");
        };
        let params: PublishDiagnosticsParams = serde_json::from_value(notification.params).unwrap();
        assert_eq!(params.diagnostics.len(), 2);
        assert!(params.diagnostics.iter().all(|diagnostic| {
            diagnostic.code
                == Some(lsp_types::NumberOrString::String(
                    "unresolved-reference".to_string(),
                ))
        }));
        assert_eq!(
            params
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.range)
                .collect::<Vec<_>>(),
            vec![
                Range::new(Position::new(0, 5), Position::new(0, 12)),
                Range::new(Position::new(0, 21), Position::new(0, 28)),
            ]
        );
    }

    #[test]
    fn diagnostics_publish_duplicate_broken_and_ambiguous_id_codes() {
        let source = "first\n--^ id: same\nsecond\n--^ id: same\n[[@same]] [[@missing]]\n";
        let documents = BTreeMap::from([(PathBuf::from("open.maki"), source.to_string())]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::from([PathBuf::from("open.maki")]),
            documents,
        };
        let (server_connection, client_connection) = Connection::memory();

        server.publish_diagnostics(&server_connection).unwrap();

        let Message::Notification(notification) = client_connection.receiver.recv().unwrap() else {
            panic!("expected a diagnostics notification");
        };
        let params: PublishDiagnosticsParams = serde_json::from_value(notification.params).unwrap();
        let codes = params
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic.code.as_ref()? {
                lsp_types::NumberOrString::String(code) => Some(code.as_str()),
                lsp_types::NumberOrString::Number(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            codes,
            vec![
                "duplicate-id",
                "duplicate-id",
                "ambiguous-id-link",
                "broken-id-link"
            ]
        );
    }

    #[test]
    fn document_symbols_follow_heading_hierarchy_and_cover_sections() {
        let source = "= Parent\nintro\n== Child 😀\nbody\n= Sibling\nend\n";
        let analysis = maki_core::analysis::analyze_document(Path::new("index.maki"), source);
        let symbols = heading_symbols(source, &analysis.headings);

        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Parent");
        assert_eq!(
            symbols[0].range,
            Range::new(Position::new(0, 2), Position::new(4, 0))
        );
        assert_eq!(
            symbols[0].selection_range,
            Range::new(Position::new(0, 2), Position::new(0, 8))
        );

        let children = symbols[0]
            .children
            .as_ref()
            .expect("Parent should contain Child");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Child 😀");
        assert_eq!(
            children[0].range,
            Range::new(Position::new(2, 3), Position::new(4, 0))
        );

        assert_eq!(symbols[1].name, "Sibling");
        assert_eq!(
            symbols[1].range,
            Range::new(Position::new(4, 2), Position::new(6, 0))
        );
    }

    #[test]
    fn workspace_symbol_search_finds_headings_case_insensitively() {
        let documents = BTreeMap::from([(
            PathBuf::from("notes/alpha.maki"),
            "--^ title: Alpha\n\n= Overview\n== Details\n".to_string(),
        )]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        };
        let params = WorkspaceSymbolParams {
            query: "DETAIL".to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let WorkspaceSymbolResponse::Flat(symbols) = server.workspace_symbols(params) else {
            panic!("workspace symbols should use the flat response form");
        };

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Details");
        assert_eq!(symbols[0].kind, SymbolKind::NAMESPACE);
        assert_eq!(symbols[0].container_name.as_deref(), Some("Alpha"));
        assert_eq!(
            symbols[0].location.uri.as_str(),
            "file:///workspace/notes/alpha.maki"
        );
        assert_eq!(
            symbols[0].location.range,
            Range::new(Position::new(3, 3), Position::new(3, 10))
        );
    }

    #[test]
    fn workspace_symbol_search_exposes_document_local_ids() {
        let documents = BTreeMap::from([(
            PathBuf::from("notes/alpha.maki"),
            "--^ title: Alpha\n\nschedule\n--^ id: my-schedule\n".to_string(),
        )]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
            open_documents: BTreeSet::new(),
            documents,
        };
        let params = WorkspaceSymbolParams {
            query: "SCHEDULE".to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let WorkspaceSymbolResponse::Flat(symbols) = server.workspace_symbols(params) else {
            panic!("workspace symbols should use the flat response form");
        };

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "my-schedule");
        assert_eq!(symbols[0].kind, SymbolKind::KEY);
        assert_eq!(symbols[0].container_name.as_deref(), Some("Alpha"));
        assert_eq!(
            symbols[0].location.range,
            Range::new(Position::new(3, 8), Position::new(3, 19))
        );
    }
}
