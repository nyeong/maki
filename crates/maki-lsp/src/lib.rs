#![allow(deprecated)]

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionOptions, CompletionParams, CompletionResponse, Diagnostic,
    DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentLink, DocumentLinkOptions, DocumentLinkParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, InitializeParams, Location,
    MarkupContent, MarkupKind, OneOf, Position, PositionEncodingKind, PublishDiagnosticsParams,
    Range, ServerCapabilities, SymbolInformation, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use maki_core::analysis::{
    AnalysisDiagnosticKind, DateOrigin, DefinitionTarget, DocumentAnalysis, HeadingOccurrence,
    LinkResolution, ProjectAnalysis, SourceSnapshot, analyze_project, property_description,
};
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
    analysis: ProjectAnalysis,
}

impl Server {
    fn new(workspace_root: PathBuf) -> LspResult<Self> {
        let source_root = source_root(&workspace_root)?;
        let documents = load_documents(&source_root)?;
        let analysis = analyze_documents(&documents);

        Ok(Self {
            source_root,
            documents,
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
            "initialized" => {
                self.publish_diagnostics(connection)?;
            }
            "textDocument/didOpen" => {
                let params: DidOpenTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                if let Some(path) = self.relative_path(&params.text_document.uri) {
                    self.documents.insert(path, params.text_document.text);
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
                ) {
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
                            self.documents.insert(path, source);
                        }
                        Err(_) => {
                            self.documents.remove(&path);
                        }
                    }
                    self.reanalyze();
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
        for (path, source) in &self.documents {
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
            let uri = Url::from_file_path(self.source_root.join(path))
                .map_err(|_| "failed to create document URI")?;
            let params = PublishDiagnosticsParams::new(uri, diagnostics, None);
            connection
                .sender
                .send(Message::Notification(Notification::new(
                    "textDocument/publishDiagnostics".to_string(),
                    params,
                )))?;
        }
        Ok(())
    }

    fn definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let (source, document) = self.document_for_uri(uri)?;
        let offset = lsp_offset(source, params.text_document_position_params.position)?;
        let occurrence = document
            .note_links
            .iter()
            .find(|occurrence| span_touches(occurrence.span, offset))?;
        let LinkResolution::Found(target) = occurrence.resolution.as_ref()? else {
            return None;
        };
        Some(GotoDefinitionResponse::Scalar(self.location(target)?))
    }

    fn location(&self, target: &DefinitionTarget) -> Option<Location> {
        self.symbol_location(&target.path, target.selection_span)
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
        let (source, _) = self.document_for_uri(uri)?;
        let offset = lsp_offset(source, params.text_document_position.position)?;
        Some(CompletionResponse::Array(completion_items(
            &self.analysis,
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

fn completion_items(project: &ProjectAnalysis, source: &str, offset: usize) -> Vec<CompletionItem> {
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
        let target = &before_cursor[open + 2..];
        if let Some((_, heading_prefix)) = target.split_once('#') {
            return project
                .documents
                .values()
                .flat_map(|document| document.headings.iter())
                .filter(|heading| heading.anchor.starts_with(heading_prefix))
                .map(|heading| CompletionItem {
                    label: heading.anchor.clone(),
                    detail: Some("Maki heading".to_string()),
                    ..CompletionItem::default()
                })
                .collect();
        }
        return project
            .note_candidates()
            .filter(|document| {
                document.canonical_path.starts_with(target) || document.title.starts_with(target)
            })
            .map(|document| CompletionItem {
                label: document.canonical_path.clone(),
                detail: Some(document.title.clone()),
                ..CompletionItem::default()
            })
            .collect();
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
            let fragment = target
                .heading_anchor
                .as_deref()
                .map_or(String::new(), |anchor| format!("#{anchor}"));
            format!("Resolves to `{}`{fragment}.", target.path.display())
        }
        Some(LinkResolution::BrokenNote) => "Target note was not found.".to_string(),
        Some(LinkResolution::AmbiguousNote) => "Target note is ambiguous.".to_string(),
        Some(LinkResolution::BrokenHeading) => "Target heading was not found.".to_string(),
        Some(LinkResolution::AmbiguousHeading) => "Target heading is ambiguous.".to_string(),
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
    span.start <= offset && offset <= span.end
}

fn diagnostic_code(kind: AnalysisDiagnosticKind) -> &'static str {
    match kind {
        AnalysisDiagnosticKind::ParseWarning => "parse-warning",
        AnalysisDiagnosticKind::BrokenNoteLink => "broken-note-link",
        AnalysisDiagnosticKind::AmbiguousNoteLink => "ambiguous-note-link",
        AnalysisDiagnosticKind::BrokenHeadingLink => "broken-heading-link",
        AnalysisDiagnosticKind::AmbiguousHeadingLink => "ambiguous-heading-link",
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
        let notes = completion_items(&project, "[[ot", 4);
        let headings = completion_items(&project, "[[other#H", 9);
        let properties = completion_items(&project, "--v ti", 6);

        assert!(notes.iter().any(|item| item.label == "other"));
        assert!(headings.iter().any(|item| item.label == "Heading"));
        assert!(properties.iter().any(|item| item.label == "title"));
    }

    #[test]
    fn completion_offers_current_quote_modes() {
        let project = analyze_documents(&BTreeMap::new());
        let source = "--v mode: ";
        let modes = completion_items(&project, source, source.len())
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();

        assert_eq!(modes, vec!["block", "pre", "text"]);
    }

    #[test]
    fn document_links_open_url_reference_markers() {
        let source = r#"- [요카토 추천 리스트]
  - [김치]
- [로컬]

[요카토 추천 리스트]: https://docs.google.com/document/d/example/edit
[김치]: https://hakkeido.com/
[로컬]: other
"#;
        let documents = BTreeMap::from([(PathBuf::from("index.maki"), source.to_string())]);
        let server = Server {
            source_root: PathBuf::from("/workspace"),
            analysis: analyze_documents(&documents),
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
            Range::new(Position::new(1, 4), Position::new(1, 8))
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
}
