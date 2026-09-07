use std::collections::{HashMap, HashSet};

use lsp_types::{
    CodeAction, CodeActionContext, CodeActionDisabled, CodeActionKind, CodeActionOrCommand,
    CodeActionResponse, DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, Position,
    Range, TextDocumentEdit, TextEdit, Url, WorkspaceEdit,
};
use maki_core::analysis::{
    AnalysisBlockKind, DocumentAnalysis, ReferenceDefinitionOccurrence, ReferenceDefinitionState,
    UrlLinkOccurrence,
};
use maki_core::parser::ReferenceValueKind;
use maki_core::source::{SourceMap, SourceSpan, Utf16Position};
use serde::{Deserialize, Serialize};

use crate::page_title::PageTitleProvider;
#[cfg(test)]
use crate::{lsp_offset, lsp_range};

const ACTION_DATA_VERSION: u8 = 1;
const EXTRACT_ACTION_TITLE: &str = "Extract URL as reference";
const FETCH_EXTRACT_ACTION_TITLE: &str = "Fetch page title and extract URL as reference";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExtractionCapabilities {
    pub(crate) document_changes: bool,
    pub(crate) code_action_literals: bool,
    pub(crate) lazy_code_action_edits: bool,
    pub(crate) disabled_code_actions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ExtractMode {
    Named,
    BareExisting,
    BareFetch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtractActionData {
    version: u8,
    uri: Url,
    span_start: usize,
    span_end: usize,
    target: String,
    mode: ExtractMode,
}

struct ExtractionIndex<'a> {
    reserved_keys: HashSet<&'a str>,
    active_url_definitions: HashMap<&'a str, &'a ReferenceDefinitionOccurrence>,
}

struct ExtractionContext<'a> {
    uri: &'a Url,
    source: &'a str,
    source_map: SourceMap<'a>,
    document: &'a DocumentAnalysis,
    document_version: Option<i32>,
    capabilities: ExtractionCapabilities,
}

impl<'a> ExtractionIndex<'a> {
    fn new(document: &'a DocumentAnalysis) -> Self {
        let mut reserved_keys = HashSet::new();
        let mut active_url_definitions = HashMap::new();

        for definition in &document.reference_graph.definitions {
            reserved_keys.insert(definition.key.as_str());
            if definition.state == ReferenceDefinitionState::Active
                && definition.value_kind == ReferenceValueKind::HyperLink
                && let Some(target) = definition.semantic_target.as_deref()
            {
                active_url_definitions.entry(target).or_insert(definition);
            }
        }

        Self {
            reserved_keys,
            active_url_definitions,
        }
    }

    fn active_url_definition(&self, target: &str) -> Option<&'a ReferenceDefinitionOccurrence> {
        self.active_url_definitions.get(target).copied()
    }
}

pub(crate) fn extract_url_actions(
    uri: &Url,
    source: &str,
    document: &DocumentAnalysis,
    document_version: Option<i32>,
    capabilities: ExtractionCapabilities,
    range: Range,
    context: &CodeActionContext,
) -> CodeActionResponse {
    if !capabilities.code_action_literals || !context_allows_extract(context) {
        return Vec::new();
    }

    let source_map = SourceMap::new(source);
    let Some(selection) = selection_span(&source_map, range) else {
        return Vec::new();
    };
    let index = ExtractionIndex::new(document);
    let extraction = ExtractionContext {
        uri,
        source,
        source_map,
        document,
        document_version,
        capabilities,
    };

    document
        .url_links
        .iter()
        .filter(|link| spans_intersect_or_touch(link.span, selection))
        .filter(|link| parse_http_url(&link.target).is_some())
        .filter_map(|link| action_for_link(&extraction, link, &index))
        .map(CodeActionOrCommand::CodeAction)
        .collect()
}

pub(crate) fn resolve_extract_url_action<'a>(
    mut action: CodeAction,
    document_for_uri: impl FnOnce(&Url) -> Option<(&'a str, &'a DocumentAnalysis, Option<i32>)>,
    page_titles: &dyn PageTitleProvider,
    capabilities: ExtractionCapabilities,
) -> CodeAction {
    // A client returns the whole action to the server. Never trust a supplied edit
    // or command when resolving it; reconstruct the edit from current analysis.
    action.edit = None;
    action.command = None;
    action.disabled = None;

    if action.kind.as_ref() != Some(&CodeActionKind::REFACTOR_EXTRACT) {
        return disabled(
            action,
            "This is not a Maki URL extraction action.",
            capabilities,
        );
    }

    let Some(data) = action
        .data
        .clone()
        .and_then(|value| serde_json::from_value::<ExtractActionData>(value).ok())
        .filter(|data| data.version == ACTION_DATA_VERSION)
    else {
        return disabled(
            action,
            "The URL extraction action data is invalid.",
            capabilities,
        );
    };

    let expected_title = match data.mode {
        ExtractMode::BareFetch => FETCH_EXTRACT_ACTION_TITLE,
        ExtractMode::Named | ExtractMode::BareExisting => EXTRACT_ACTION_TITLE,
    };
    if action.title != expected_title || data.span_start > data.span_end {
        return disabled(
            action,
            "The URL extraction action data is invalid.",
            capabilities,
        );
    }
    if data.mode == ExtractMode::BareFetch
        && (!capabilities.document_changes
            || !capabilities.code_action_literals
            || !capabilities.lazy_code_action_edits)
    {
        return disabled(
            action,
            "Fetching a page title requires versioned document edits.",
            capabilities,
        );
    }

    let Some((source, document, document_version)) = document_for_uri(&data.uri) else {
        return disabled(
            action,
            "The Maki document is no longer available.",
            capabilities,
        );
    };
    if data.mode == ExtractMode::BareFetch && document_version.is_none() {
        return disabled(
            action,
            "Fetching a page title requires an open, versioned document.",
            capabilities,
        );
    }
    let span = SourceSpan::new(data.span_start, data.span_end);
    let Some(link) = document.url_links.iter().find(|link| {
        link.span == span
            && link.target == data.target
            && source.get(link.span.start..link.span.end).is_some()
    }) else {
        return disabled(
            action,
            "The URL link changed before the action was applied.",
            capabilities,
        );
    };

    let mode_matches_link = match data.mode {
        ExtractMode::Named => link.title.is_some(),
        ExtractMode::BareExisting | ExtractMode::BareFetch => link.title.is_none(),
    };
    if !mode_matches_link {
        return disabled(
            action,
            "The URL link changed before the action was applied.",
            capabilities,
        );
    }

    let Some(parsed_url) = parse_http_url(&link.target) else {
        return disabled(action, "The URL link is no longer valid.", capabilities);
    };
    let index = ExtractionIndex::new(document);
    let extraction = ExtractionContext {
        uri: &data.uri,
        source,
        source_map: SourceMap::new(source),
        document,
        document_version,
        capabilities,
    };

    let display = match data.mode {
        ExtractMode::Named => link.title.clone(),
        ExtractMode::BareExisting => {
            if index.active_url_definition(&link.target).is_none() {
                return disabled(
                    action,
                    "The matching Reference Definition no longer exists.",
                    capabilities,
                );
            }
            valid_reference_component(fallback_url_display(&link.target))
                .then(|| fallback_url_display(&link.target).to_string())
        }
        ExtractMode::BareFetch => {
            if index.active_url_definition(&link.target).is_some() {
                valid_reference_component(fallback_url_display(&link.target))
                    .then(|| fallback_url_display(&link.target).to_string())
            } else {
                let fetched_title = page_titles.page_title(&parsed_url);
                fetched_title
                    .filter(|title| valid_fetched_title(title))
                    .or_else(|| {
                        let fallback = fallback_url_display(&link.target);
                        valid_reference_component(fallback).then(|| fallback.to_string())
                    })
            }
        }
    };

    let Some(display) = display else {
        return disabled(
            action,
            "The URL has no title that can be represented as a Maki reference.",
            capabilities,
        );
    };
    let Some(edit) = extraction_edit(&extraction, link, &display, &index) else {
        return disabled(
            action,
            "The URL title cannot be represented as a Maki reference.",
            capabilities,
        );
    };

    action.edit = Some(edit);
    action
}

fn action_for_link(
    context: &ExtractionContext<'_>,
    link: &UrlLinkOccurrence,
    index: &ExtractionIndex<'_>,
) -> Option<CodeAction> {
    let (title, mode, edit) = if let Some(display) = link.title.as_deref() {
        let edit = extraction_edit(context, link, display, index)?;
        (EXTRACT_ACTION_TITLE, ExtractMode::Named, Some(edit))
    } else if index.active_url_definition(&link.target).is_some() {
        let display = fallback_url_display(&link.target);
        if !valid_reference_component(display) {
            return None;
        }
        let edit = extraction_edit(context, link, display, index)?;
        (EXTRACT_ACTION_TITLE, ExtractMode::BareExisting, Some(edit))
    } else if !context.capabilities.document_changes
        || !context.capabilities.lazy_code_action_edits
        || context.document_version.is_none()
    {
        return None;
    } else {
        (FETCH_EXTRACT_ACTION_TITLE, ExtractMode::BareFetch, None)
    };

    let data = if context.capabilities.lazy_code_action_edits {
        Some(
            serde_json::to_value(ExtractActionData {
                version: ACTION_DATA_VERSION,
                uri: context.uri.clone(),
                span_start: link.span.start,
                span_end: link.span.end,
                target: link.target.clone(),
                mode,
            })
            .ok()?,
        )
    } else {
        None
    };

    Some(CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        edit,
        data,
        ..CodeAction::default()
    })
}

fn extraction_edit(
    context: &ExtractionContext<'_>,
    link: &UrlLinkOccurrence,
    display: &str,
    index: &ExtractionIndex<'_>,
) -> Option<WorkspaceEdit> {
    let source = context.source;
    let in_table_row = context.document.blocks.iter().any(|block| {
        block.kind == AnalysisBlockKind::Table && block.span.contains(link.span.start)
    });
    if !valid_reference_component(display)
        || (in_table_row && !is_table_safe_reference_component(display))
    {
        return None;
    }

    let reusable_definition = index
        .active_url_definition(&link.target)
        .filter(|definition| !in_table_row || is_table_safe_reference_component(&definition.key));
    let (key, definition) = if let Some(existing) = reusable_definition {
        (existing.key.clone(), None)
    } else {
        let key = available_key(display, &index.reserved_keys);
        let definition = format!("[{key}]: <{}>", link.target);
        (key, Some(definition))
    };

    let reference = if display == key {
        format!("[{display}][]")
    } else {
        format!("[{display}][{key}]")
    };
    let mut edits = vec![TextEdit {
        range: source_map_range(&context.source_map, link.span)?,
        new_text: reference,
    }];

    if let Some(definition) = definition {
        let (offset, new_text) = definition_insertion(source, context.document, &definition);
        edits.push(TextEdit {
            range: source_map_range(&context.source_map, SourceSpan::new(offset, offset))?,
            new_text,
        });
    }

    if context.capabilities.document_changes {
        Some(WorkspaceEdit {
            changes: None,
            document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: context.uri.clone(),
                    version: context.document_version,
                },
                edits: edits.into_iter().map(OneOf::Left).collect(),
            }])),
            change_annotations: None,
        })
    } else {
        Some(WorkspaceEdit::new(HashMap::from([(
            context.uri.clone(),
            edits,
        )])))
    }
}

fn is_table_safe_reference_component(value: &str) -> bool {
    let mut backslashes = 0usize;

    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '|' && backslashes.is_multiple_of(2) {
            return false;
        }
        backslashes = 0;
    }

    true
}

fn available_key(base: &str, reserved: &HashSet<&str>) -> String {
    if !reserved.contains(base) {
        return base.to_string();
    }

    for suffix in 2usize.. {
        let candidate = format!("{base}-{suffix}");
        if !reserved.contains(candidate.as_str()) {
            return candidate;
        }
    }

    unreachable!("the numeric suffix space is not finite")
}

fn definition_insertion(
    source: &str,
    document: &DocumentAnalysis,
    definition: &str,
) -> (usize, String) {
    let line_ending = line_ending(source);
    if let Some(trailing) = document
        .reference_graph
        .definitions
        .last()
        .filter(|definition| {
            source
                .get(definition.definition_span.end..)
                .is_some_and(|rest| rest.trim().is_empty())
        })
    {
        return (
            trailing.definition_span.end,
            format!("{line_ending}{definition}"),
        );
    }

    let had_final_newline = source.ends_with('\n');
    let prefix = if source.is_empty() || ends_with_blank_line(source, line_ending) {
        ""
    } else if had_final_newline {
        line_ending
    } else {
        // A Reference Definition is a root block. Keep it separate from the
        // preceding block when adding it to a non-empty document.
        return (
            source.len(),
            format!("{line_ending}{line_ending}{definition}"),
        );
    };
    let suffix = if had_final_newline { line_ending } else { "" };
    (source.len(), format!("{prefix}{definition}{suffix}"))
}

fn line_ending(source: &str) -> &'static str {
    source
        .find('\n')
        .filter(|newline| *newline > 0 && source.as_bytes()[newline - 1] == b'\r')
        .map_or("\n", |_| "\r\n")
}

fn ends_with_blank_line(source: &str, line_ending: &str) -> bool {
    source
        .strip_suffix(line_ending)
        .is_some_and(|without_last| without_last.ends_with(line_ending))
}

fn fallback_url_display(target: &str) -> &str {
    let Some((scheme, body)) = target.split_once("://") else {
        return target;
    };
    if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        body
    } else {
        target
    }
}

fn valid_reference_component(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.starts_with('^')
        && !value.contains(['[', ']'])
        && !value.chars().any(char::is_control)
}

fn valid_fetched_title(value: &str) -> bool {
    valid_reference_component(value) && !value.contains('|')
}

fn parse_http_url(target: &str) -> Option<Url> {
    let url = Url::parse(target).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn context_allows_extract(context: &CodeActionContext) -> bool {
    context.only.as_ref().is_none_or(|only| {
        only.iter()
            .any(|requested| action_kind_includes(requested, &CodeActionKind::REFACTOR_EXTRACT))
    })
}

fn action_kind_includes(requested: &CodeActionKind, provided: &CodeActionKind) -> bool {
    let requested = requested.as_str();
    let provided = provided.as_str();
    requested.is_empty()
        || requested == provided
        || provided
            .strip_prefix(requested)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn selection_span(source_map: &SourceMap<'_>, range: Range) -> Option<SourceSpan> {
    let offset = |position: Position| {
        source_map.offset_utf16(Utf16Position {
            line: position.line as usize,
            character: position.character as usize,
        })
    };
    let start = offset(range.start)?;
    let end = offset(range.end)?;
    (start <= end).then(|| SourceSpan::new(start, end))
}

fn source_map_range(source_map: &SourceMap<'_>, span: SourceSpan) -> Option<Range> {
    let position = |offset| {
        let position = source_map.utf16_position(offset)?;
        Some(Position::new(
            position.line as u32,
            position.character as u32,
        ))
    };
    Some(Range::new(position(span.start)?, position(span.end)?))
}

fn spans_intersect_or_touch(left: SourceSpan, right: SourceSpan) -> bool {
    left.start <= right.end && right.start <= left.end
}

fn disabled(
    mut action: CodeAction,
    reason: &str,
    capabilities: ExtractionCapabilities,
) -> CodeAction {
    action.edit = None;
    action.command = None;
    action.disabled = capabilities
        .disabled_code_actions
        .then(|| CodeActionDisabled {
            reason: reason.to_string(),
        });
    action
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use lsp_types::{CodeActionContext, CodeActionKind, Position};
    use maki_core::analysis::analyze_document;

    use super::*;

    struct FakePageTitles {
        calls: AtomicUsize,
        title: Option<String>,
    }

    impl FakePageTitles {
        fn new(title: Option<&str>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                title: title.map(str::to_string),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl PageTitleProvider for FakePageTitles {
        fn page_title(&self, _url: &Url) -> Option<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.title.clone()
        }
    }

    fn uri() -> Url {
        Url::parse("file:///workspace/index.maki").unwrap()
    }

    fn context(only: Option<Vec<CodeActionKind>>) -> CodeActionContext {
        CodeActionContext {
            diagnostics: Vec::new(),
            only,
            trigger_kind: None,
        }
    }

    fn whole_document_range(source: &str) -> Range {
        lsp_range(source, SourceSpan::new(0, source.len())).unwrap()
    }

    fn capabilities(
        document_changes: bool,
        code_action_literals: bool,
        lazy_code_action_edits: bool,
    ) -> ExtractionCapabilities {
        ExtractionCapabilities {
            document_changes,
            code_action_literals,
            lazy_code_action_edits,
            disabled_code_actions: true,
        }
    }

    fn actions(source: &str, range: Range) -> Vec<CodeAction> {
        let document = analyze_document(Path::new("index.maki"), source);
        extract_url_actions(
            &uri(),
            source,
            &document,
            Some(7),
            capabilities(true, true, true),
            range,
            &context(None),
        )
        .into_iter()
        .map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action,
            CodeActionOrCommand::Command(_) => panic!("expected a literal Code Action"),
        })
        .collect()
    }

    fn one_action(source: &str) -> CodeAction {
        let mut actions = actions(source, whole_document_range(source));
        assert_eq!(actions.len(), 1);
        actions.remove(0)
    }

    fn resolve(source: &str, action: CodeAction, provider: &dyn PageTitleProvider) -> CodeAction {
        let document = analyze_document(Path::new("index.maki"), source);
        resolve_extract_url_action(
            action,
            |requested| (requested == &uri()).then_some((source, &document, Some(7))),
            provider,
            capabilities(true, true, true),
        )
    }

    fn apply_action(source: &str, action: &CodeAction) -> String {
        let edit = action.edit.as_ref().expect("action should contain an edit");
        let edits = if let Some(changes) = edit.changes.as_ref() {
            changes
                .get(&uri())
                .expect("action should edit the current document")
                .iter()
                .collect::<Vec<_>>()
        } else {
            let Some(DocumentChanges::Edits(documents)) = edit.document_changes.as_ref() else {
                panic!("action should contain text document edits");
            };
            let document = documents
                .iter()
                .find(|document| document.text_document.uri == uri())
                .expect("action should edit the current document");
            document
                .edits
                .iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    OneOf::Right(_) => panic!("action should not use annotated edits"),
                })
                .collect::<Vec<_>>()
        };
        let mut edits = edits
            .iter()
            .map(|edit| {
                let start = lsp_offset(source, edit.range.start).unwrap();
                let end = lsp_offset(source, edit.range.end).unwrap();
                (start, end, edit.new_text.as_str())
            })
            .collect::<Vec<_>>();
        edits.sort_by_key(|edit| std::cmp::Reverse((edit.0, edit.1)));

        let mut result = source.to_string();
        for (start, end, replacement) in edits {
            result.replace_range(start..end, replacement);
        }
        result
    }

    fn action_document_version(action: &CodeAction) -> Option<i32> {
        let Some(DocumentChanges::Edits(documents)) = action
            .edit
            .as_ref()
            .and_then(|edit| edit.document_changes.as_ref())
        else {
            return None;
        };
        documents.first()?.text_document.version
    }

    #[test]
    fn named_url_has_an_immediate_edit_and_never_fetches() {
        let source = "[사이트]<https://example.com/path>";
        let action = one_action(source);
        assert_eq!(action.title, EXTRACT_ACTION_TITLE);
        assert_eq!(action.kind, Some(CodeActionKind::REFACTOR_EXTRACT));
        assert!(action.edit.is_some());
        assert_eq!(
            apply_action(source, &action),
            "[사이트][]\n\n[사이트]: <https://example.com/path>"
        );

        let provider = FakePageTitles::new(Some("Must not be used"));
        let resolved = resolve(source, action, &provider);
        assert_eq!(provider.calls(), 0);
        assert!(resolved.disabled.is_none());
        assert_eq!(
            apply_action(source, &resolved),
            "[사이트][]\n\n[사이트]: <https://example.com/path>"
        );
    }

    #[test]
    fn bare_url_fetches_only_when_the_listed_action_is_resolved() {
        let source = "See <https://example.com/path>.";
        let action = one_action(source);
        let provider = FakePageTitles::new(Some("Example Domain"));

        assert_eq!(action.title, FETCH_EXTRACT_ACTION_TITLE);
        assert!(action.edit.is_none());
        assert_eq!(provider.calls(), 0, "listing must not perform network I/O");

        let resolved = resolve(source, action, &provider);
        assert_eq!(provider.calls(), 1);
        assert_eq!(action_document_version(&resolved), Some(7));
        assert_eq!(
            apply_action(source, &resolved),
            "See [Example Domain][].\n\n[Example Domain]: <https://example.com/path>"
        );
    }

    #[test]
    fn client_capabilities_gate_lazy_fetch_but_keep_immediate_extractions() {
        let source = "<https://example.com/>";
        let document = analyze_document(Path::new("index.maki"), source);
        let range = whole_document_range(source);

        for (document_changes, literals, lazy_edits) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(
                extract_url_actions(
                    &uri(),
                    source,
                    &document,
                    Some(7),
                    capabilities(document_changes, literals, lazy_edits),
                    range,
                    &context(None),
                )
                .is_empty()
            );
        }

        let named = "[Example]<https://example.com/>";
        let document = analyze_document(Path::new("index.maki"), named);
        let action = extract_url_actions(
            &uri(),
            named,
            &document,
            Some(7),
            capabilities(false, true, false),
            whole_document_range(named),
            &context(None),
        )
        .into_iter()
        .next()
        .and_then(|action| match action {
            CodeActionOrCommand::CodeAction(action) => Some(action),
            CodeActionOrCommand::Command(_) => None,
        })
        .expect("named extraction should remain available");
        assert!(action.data.is_none());
        assert!(
            action
                .edit
                .as_ref()
                .is_some_and(|edit| edit.changes.is_some())
        );

        assert!(
            extract_url_actions(
                &uri(),
                source,
                &analyze_document(Path::new("index.maki"), source),
                None,
                capabilities(true, true, true),
                range,
                &context(None),
            )
            .is_empty(),
            "a title fetch must not be offered without an open document version"
        );

        let existing = "<https://example.com/>\n\n[site]: <https://example.com/>\n";
        let document = analyze_document(Path::new("index.maki"), existing);
        assert_eq!(
            extract_url_actions(
                &uri(),
                existing,
                &document,
                Some(7),
                capabilities(false, true, false),
                whole_document_range(existing),
                &context(None),
            )
            .len(),
            1,
            "reusing an existing definition requires no lazy resolve"
        );

        assert!(
            extract_url_actions(
                &uri(),
                named,
                &analyze_document(Path::new("index.maki"), named),
                Some(7),
                capabilities(true, false, true),
                whole_document_range(named),
                &context(None),
            )
            .is_empty(),
            "clients without Code Action literal support cannot receive a literal action"
        );
    }

    #[test]
    fn escaped_pipe_titles_remain_extractable_inside_tables() {
        let table = "| Link |\n|---|\n| [Left \\| Right]<https://example.com/> |";
        let action = one_action(table);
        assert_eq!(
            apply_action(table, &action),
            "| Link |\n|---|\n| [Left \\| Right][] |\n\n[Left \\| Right]: <https://example.com/>"
        );

        for safe in ["plain", r"a\|b", r"a\\\|b"] {
            assert!(is_table_safe_reference_component(safe));
        }
        for unsafe_value in ["a|b", r"a\\|b", r"a\\\\|b"] {
            assert!(!is_table_safe_reference_component(unsafe_value));
        }
    }

    #[test]
    fn table_safety_uses_parsed_blocks_instead_of_a_line_prefix_heuristic() {
        let paragraph = "| [a|b]<https://example.com/>";
        let action = one_action(paragraph);
        assert_eq!(
            apply_action(paragraph, &action),
            "| [a|b][]\n\n[a|b]: <https://example.com/>"
        );

        let target = "https://example.com/";
        let paragraph = format!("| [Shown]<{target}>\n\n[a|b]: <{target}>");
        let action = one_action(&paragraph);
        assert_eq!(
            apply_action(&paragraph, &action),
            format!("| [Shown][a|b]\n\n[a|b]: <{target}>")
        );
    }

    #[test]
    fn unrepresentable_immediate_extractions_are_not_offered() {
        let ipv6 = "<https://[2606:4700:4700::1111]/>\n\n[site]: <https://[2606:4700:4700::1111]/>";
        assert!(
            actions(ipv6, whole_document_range(ipv6)).is_empty(),
            "an unrepresentable fallback display must not produce an edit-less action"
        );
    }

    #[test]
    fn active_exact_url_definition_is_reused_without_fetching() {
        let source = "See <https://example.com/path>.\n\n[site]: <https://example.com/path>\n";
        let action = one_action(source);
        let provider = FakePageTitles::new(Some("Must not be used"));

        assert_eq!(action.title, EXTRACT_ACTION_TITLE);
        assert!(action.edit.is_some());
        assert_eq!(
            apply_action(source, &action),
            "See [example.com/path][site].\n\n[site]: <https://example.com/path>\n"
        );

        let resolved = resolve(source, action, &provider);
        assert_eq!(provider.calls(), 0);
        assert_eq!(
            apply_action(source, &resolved),
            "See [example.com/path][site].\n\n[site]: <https://example.com/path>\n"
        );
    }

    #[test]
    fn resolve_rechecks_new_exact_definition_before_fetching() {
        let listed_source = "<https://example.com/path>";
        let action = one_action(listed_source);
        let current_source = "<https://example.com/path>\n\n[site]: <https://example.com/path>\n";
        let provider = FakePageTitles::new(Some("Must not be used"));

        let resolved = resolve(current_source, action, &provider);
        assert_eq!(provider.calls(), 0);
        assert_eq!(
            apply_action(current_source, &resolved),
            "[example.com/path][site]\n\n[site]: <https://example.com/path>\n"
        );
    }

    #[test]
    fn inactive_duplicate_url_is_not_reused_and_all_keys_reserve_suffixes() {
        let source = "<https://second.example/>\n\n[Title]: <https://first.example/>\n[Title]: <https://second.example/>\n[Title-2]: prose\n";
        let action = one_action(source);
        let provider = FakePageTitles::new(Some("Title"));

        assert!(action.edit.is_none());
        let resolved = resolve(source, action, &provider);
        assert_eq!(provider.calls(), 1);
        assert_eq!(
            apply_action(source, &resolved),
            "[Title][Title-3]\n\n[Title]: <https://first.example/>\n[Title]: <https://second.example/>\n[Title-2]: prose\n[Title-3]: <https://second.example/>\n"
        );
    }

    #[test]
    fn dense_key_collisions_are_indexed_once() {
        let mut owned = vec!["Title".to_string()];
        owned.extend((2..=10_000).map(|suffix| format!("Title-{suffix}")));
        let reserved = owned.iter().map(String::as_str).collect::<HashSet<_>>();

        assert_eq!(available_key("Title", &reserved), "Title-10001");
    }

    #[test]
    fn utf16_ranges_and_touching_selections_choose_only_the_intended_url() {
        let source = "한글 😀 [제목]<https://named.example/> 뒤 <https://bare.example/>";
        let document = analyze_document(Path::new("index.maki"), source);
        let named = &document.url_links[0];
        let named_range = lsp_range(source, named.span).unwrap();
        assert_eq!(named_range.start, Position::new(0, 6));

        let at_end = Range::new(named_range.end, named_range.end);
        let found = extract_url_actions(
            &uri(),
            source,
            &document,
            Some(7),
            capabilities(true, true, true),
            at_end,
            &context(None),
        );
        assert_eq!(found.len(), 1, "a zero-width range may touch the link end");

        let before_and_named = Range::new(Position::new(0, 0), named_range.end);
        let found = extract_url_actions(
            &uri(),
            source,
            &document,
            Some(7),
            capabilities(true, true, true),
            before_and_named,
            &context(None),
        );
        assert_eq!(found.len(), 1);

        let inner_named = Range::new(
            Position::new(0, named_range.start.character + 1),
            Position::new(0, named_range.end.character - 1),
        );
        assert_eq!(
            extract_url_actions(
                &uri(),
                source,
                &document,
                Some(7),
                capabilities(true, true, true),
                inner_named,
                &context(None),
            )
            .len(),
            1,
            "a nonzero range inside the construct selects the named URL"
        );

        let gap_offset = source.find("뒤").unwrap();
        let gap_cursor = lsp_range(source, SourceSpan::new(gap_offset, gap_offset))
            .unwrap()
            .start;
        assert!(
            extract_url_actions(
                &uri(),
                source,
                &document,
                Some(7),
                capabilities(true, true, true),
                Range::new(gap_cursor, gap_cursor),
                &context(None),
            )
            .is_empty()
        );
    }

    #[test]
    fn context_only_accepts_extract_and_its_parent_but_not_other_kinds() {
        let source = "[site]<https://example.com/>";
        let document = analyze_document(Path::new("index.maki"), source);
        let range = whole_document_range(source);

        for only in [
            None,
            Some(vec![CodeActionKind::REFACTOR]),
            Some(vec![CodeActionKind::REFACTOR_EXTRACT]),
        ] {
            assert_eq!(
                extract_url_actions(
                    &uri(),
                    source,
                    &document,
                    Some(7),
                    capabilities(true, true, true),
                    range,
                    &context(only),
                )
                .len(),
                1
            );
        }
        for only in [
            Some(vec![CodeActionKind::QUICKFIX]),
            Some(vec![CodeActionKind::REFACTOR_INLINE]),
            Some(vec![CodeActionKind::new("refactor.extract.function")]),
        ] {
            assert!(
                extract_url_actions(
                    &uri(),
                    source,
                    &document,
                    Some(7),
                    capabilities(true, true, true),
                    range,
                    &context(only),
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn crlf_and_final_newline_are_preserved_when_appending_or_extending_definitions() {
        let source = "See [새]<https://new.example/>.\r\n\r\n[old]: <https://old.example/>\r\n";
        let action = one_action(source);
        assert_eq!(
            apply_action(source, &action),
            "See [새][].\r\n\r\n[old]: <https://old.example/>\r\n[새]: <https://new.example/>\r\n"
        );

        let source = "See [New]<https://new.example/>.\r\n";
        let action = one_action(source);
        assert_eq!(
            apply_action(source, &action),
            "See [New][].\r\n\r\n[New]: <https://new.example/>\r\n"
        );

        let source = "[New]<https://new.example/>\n\n[old]: prose";
        let action = one_action(source);
        assert_eq!(
            apply_action(source, &action),
            "[New][]\n\n[old]: prose\n[New]: <https://new.example/>"
        );
    }

    #[test]
    fn raw_code_and_reference_definition_values_do_not_offer_actions() {
        let source = ": <https://code.example/>\n\n[definition]: <https://definition.example/>\n\nVisible <https://visible.example/>";
        let document = analyze_document(Path::new("index.maki"), source);
        assert_eq!(document.url_links.len(), 1);
        assert_eq!(document.url_links[0].target, "https://visible.example/");

        let found = extract_url_actions(
            &uri(),
            source,
            &document,
            Some(7),
            capabilities(true, true, true),
            whole_document_range(source),
            &context(None),
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn legacy_named_url_syntax_has_no_migration_action() {
        let source = "[old](https://old.example/) [new]<https://new.example/>";
        let document = analyze_document(Path::new("index.maki"), source);
        let found = extract_url_actions(
            &uri(),
            source,
            &document,
            Some(7),
            capabilities(true, true, true),
            whole_document_range(source),
            &context(None),
        );

        assert_eq!(found.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &found[0] else {
            panic!("expected a literal Code Action");
        };
        assert_eq!(
            apply_action(source, action),
            "[old](https://old.example/) [new][]\n\n[new]: <https://new.example/>"
        );
    }

    #[test]
    fn fetched_table_cell_title_cannot_create_a_new_cell() {
        let source = "| Link |\n|---|\n| <https://example.com/> |";
        let action = one_action(source);
        let provider = FakePageTitles::new(Some("Left ｜ Right"));
        let resolved = resolve(source, action, &provider);

        assert_eq!(provider.calls(), 1);
        assert_eq!(
            apply_action(source, &resolved),
            "| Link |\n|---|\n| [Left ｜ Right][] |\n\n[Left ｜ Right]: <https://example.com/>"
        );
    }

    #[test]
    fn table_extraction_does_not_reuse_an_exact_definition_with_a_pipe_key() {
        let target = "https://example.com/";
        let source = format!("| Link |\n|---|\n| [Shown]<{target}> |\n\n[a|b]: <{target}>");
        let action = one_action(&source);
        assert_eq!(
            apply_action(&source, &action),
            format!("| Link |\n|---|\n| [Shown][] |\n\n[a|b]: <{target}>\n[Shown]: <{target}>")
        );

        let source = format!("| Link |\n|---|\n| <{target}> |\n\n[a|b]: <{target}>");
        let action = one_action(&source);
        let provider = FakePageTitles::new(Some("Must not be used"));
        let resolved = resolve(&source, action, &provider);
        assert_eq!(provider.calls(), 0);
        assert_eq!(
            apply_action(&source, &resolved),
            format!(
                "| Link |\n|---|\n| [example.com/][] |\n\n[a|b]: <{target}>\n[example.com/]: <{target}>"
            )
        );

        let source = format!("| Link |\n|---|\n| <{target}> |\n\n[a\\|b]: <{target}>");
        let action = one_action(&source);
        assert_eq!(
            apply_action(&source, &action),
            format!("| Link |\n|---|\n| [example.com/][a\\|b] |\n\n[a\\|b]: <{target}>")
        );
    }

    #[test]
    fn stale_or_tampered_action_data_never_fetches_or_emits_an_edit() {
        let source = "<https://example.com/>";
        let original = one_action(source);
        let provider = FakePageTitles::new(Some("Example"));

        let mut tampered = original.clone();
        tampered.edit = Some(WorkspaceEdit::new(HashMap::from([(
            uri(),
            vec![TextEdit::new(
                Range::new(Position::new(0, 0), Position::new(0, 0)),
                "malicious".to_string(),
            )],
        )])));
        tampered.data.as_mut().unwrap()["target"] = serde_json::json!("https://other.example/");
        let rejected = resolve(source, tampered, &provider);
        assert!(rejected.edit.is_none());
        assert!(rejected.disabled.is_some());
        assert_eq!(provider.calls(), 0);

        let document = analyze_document(Path::new("index.maki"), source);
        let rejected = resolve_extract_url_action(
            original.clone(),
            |requested| (requested == &uri()).then_some((source, &document, None)),
            &provider,
            capabilities(true, true, true),
        );
        assert!(rejected.edit.is_none());
        assert!(rejected.disabled.is_some());
        assert_eq!(provider.calls(), 0);

        let mut without_disabled = capabilities(true, true, true);
        without_disabled.disabled_code_actions = false;
        let rejected = resolve_extract_url_action(
            original.clone(),
            |requested| (requested == &uri()).then_some((source, &document, None)),
            &provider,
            without_disabled,
        );
        assert!(rejected.edit.is_none());
        assert!(rejected.disabled.is_none());
        assert_eq!(provider.calls(), 0);

        let changed = "prefix <https://example.com/>";
        let rejected = resolve(changed, original, &provider);
        assert!(rejected.edit.is_none());
        assert!(rejected.disabled.is_some());
        assert_eq!(provider.calls(), 0);
    }

    #[test]
    fn failed_title_fetch_uses_case_insensitive_protocol_stripped_fallback() {
        let source = "<HTTPS://Example.com/Path>";
        let action = one_action(source);
        let provider = FakePageTitles::new(None);
        let resolved = resolve(source, action, &provider);

        assert_eq!(provider.calls(), 1);
        assert_eq!(
            apply_action(source, &resolved),
            "[Example.com/Path][]\n\n[Example.com/Path]: <HTTPS://Example.com/Path>"
        );
    }

    #[test]
    fn unrepresentable_fetched_title_and_fallback_disable_the_action() {
        let source = "<https://[2606:4700:4700::1111]/>";
        let action = one_action(source);
        let provider = FakePageTitles::new(Some("[invalid]"));
        let resolved = resolve(source, action, &provider);

        assert_eq!(provider.calls(), 1);
        assert!(resolved.edit.is_none());
        assert!(resolved.disabled.is_some());
    }
}
