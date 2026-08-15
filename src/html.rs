//! HTML renderer for parsed Maki documents.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    maki::{
        self, NoteLinkResolution, NoteRef, ProjectDiagnostic, ProjectDiagnosticKind,
        ProjectDiagnosticSummary, SearchEntry,
    },
    parser::{self, BlockKind, Document, Inline, ListItem, ListKind},
};

const DEFAULT_CSS: &str = include_str!("../assets/maki.css");
const SEARCH_SCRIPT: &str = include_str!("../assets/maki-search.js");
const TOC_SCRIPT: &str = include_str!("../assets/maki-toc.js");
pub(crate) const CSS_ASSET_PATH: &str = "/.maki/assets/maki.css";
pub(crate) const SEARCH_SCRIPT_ASSET_PATH: &str = "/.maki/assets/maki-search.js";
pub(crate) const TOC_SCRIPT_ASSET_PATH: &str = "/.maki/assets/maki-toc.js";
const PROJECT_NAVIGATION_HTML: &str = r#"<header class="maki-nav">
<nav aria-label="Maki navigation">
<a class="maki-home-link" href="/">/</a>
<a class="maki-meta-link" href="/@/">@</a>
<form class="maki-search" action="/.maki/search" method="get" role="search" data-maki-search>
<input class="maki-search-input" type="search" name="q" placeholder="Search title" aria-label="Search titles" autocomplete="off" spellcheck="false" data-maki-search-input>
<div class="maki-search-results" role="listbox" hidden data-maki-search-results></div>
</form>
</nav>
</header>"#;

pub(crate) struct NoteInfo {
    pub(crate) title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AssetMode {
    #[default]
    Inline,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeAsset {
    request_path: &'static str,
    file_name: &'static str,
    content_type: &'static str,
    embedded: &'static str,
}

impl RuntimeAsset {
    pub(crate) fn request_path(&self) -> &'static str {
        self.request_path
    }

    pub(crate) fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub(crate) fn embedded(&self) -> &'static str {
        self.embedded
    }

    pub(crate) fn source_path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(self.file_name)
    }
}

const RUNTIME_ASSETS: &[RuntimeAsset] = &[
    RuntimeAsset {
        request_path: CSS_ASSET_PATH,
        file_name: "maki.css",
        content_type: "text/css; charset=utf-8",
        embedded: DEFAULT_CSS,
    },
    RuntimeAsset {
        request_path: SEARCH_SCRIPT_ASSET_PATH,
        file_name: "maki-search.js",
        content_type: "application/javascript; charset=utf-8",
        embedded: SEARCH_SCRIPT,
    },
    RuntimeAsset {
        request_path: TOC_SCRIPT_ASSET_PATH,
        file_name: "maki-toc.js",
        content_type: "application/javascript; charset=utf-8",
        embedded: TOC_SCRIPT,
    },
];

pub(crate) fn runtime_assets() -> &'static [RuntimeAsset] {
    RUNTIME_ASSETS
}

pub(crate) fn runtime_asset_for_request_path(path: &str) -> Option<RuntimeAsset> {
    runtime_assets()
        .iter()
        .find(|asset| asset.request_path() == path)
        .copied()
}

fn push_stylesheet(html: &mut String, asset_mode: AssetMode) {
    match asset_mode {
        AssetMode::Inline => {
            html.push_str("<style>");
            html.push_str(DEFAULT_CSS);
            html.push_str("</style>");
        }
        AssetMode::External => {
            html.push_str("<link rel=\"stylesheet\" href=\"");
            html.push_str(CSS_ASSET_PATH);
            html.push_str("\">");
        }
    }
}

fn push_project_navigation(html: &mut String, asset_mode: AssetMode) {
    html.push_str(PROJECT_NAVIGATION_HTML);
    push_script(html, asset_mode, SEARCH_SCRIPT, SEARCH_SCRIPT_ASSET_PATH);
    push_script(html, asset_mode, TOC_SCRIPT, TOC_SCRIPT_ASSET_PATH);
}

fn push_script(html: &mut String, asset_mode: AssetMode, script: &str, asset_path: &str) {
    match asset_mode {
        AssetMode::Inline => {
            html.push_str("<script>");
            html.push_str(script);
            html.push_str("</script>");
        }
        AssetMode::External => {
            html.push_str("<script src=\"");
            html.push_str(asset_path);
            html.push_str("\"></script>");
        }
    }
}

struct Renderer<'a> {
    html: String,
    context: RenderContext<'a>,
}

#[derive(Clone, Copy)]
enum HeadingTag {
    Native(usize),
    Aria,
}

impl<'a> Renderer<'a> {
    fn render_navigation(&mut self) {
        if !self.context.project_navigation {
            return;
        }

        push_project_navigation(&mut self.html, self.context.asset_mode);
    }

    fn begin_html(&mut self, title: Option<&str>) {
        self.html = String::from(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
        );
        push_stylesheet(&mut self.html, self.context.asset_mode);
        if let Some(title) = title {
            self.html.push_str("<title>");
            self.escape_html_into(title);
            self.html.push_str("</title>");
        }
        self.html.push_str("</head><body>");
    }

    fn begin_project_page(&mut self, title: &str) {
        self.begin_html(Some(title));
        push_project_navigation(&mut self.html, self.context.asset_mode);
        self.render_heading(1, title);
    }

    fn render_anchor(&mut self, href: &str, title: &str) {
        self.html.push_str("<a");
        if maki::is_external_href(href) {
            self.html.push_str(" class=\"external-link\"");
        }
        self.html.push_str(" href=\"");
        self.escape_html_attr_into(href);
        self.html.push_str("\">");
        self.escape_html_into(title);
        self.html.push_str("</a>");
    }

    fn render_unresolved_link(&mut self, class_name: &str, title: &str, target: &str) {
        self.html.push_str("<span class=\"");
        self.escape_html_attr_into(class_name);
        self.html.push('"');
        if title != target {
            self.html.push_str(" title=\"");
            self.escape_html_attr_into(target);
            self.html.push('"');
        }
        self.html.push('>');
        self.escape_html_into(title);
        self.html.push_str("</span>");
    }

    fn render_note_link_with_title(&mut self, target: &str, title: Option<&str>) {
        let Some(context) = &self.context.project else {
            self.render_anchor(target, title.unwrap_or(target));
            return;
        };
        match (context.resolve_note_link)(target) {
            NoteLinkResolution::Found(note_ref) => {
                let note_title;
                let title = match title {
                    Some(title) => title,
                    None => {
                        note_title = (context.get_note)(&note_ref).unwrap().title;
                        &note_title
                    }
                };
                self.render_anchor(&note_ref.web_path(), title);
            }
            NoteLinkResolution::Broken => {
                self.render_unresolved_link("broken-link", title.unwrap_or(target), target);
            }
            NoteLinkResolution::Ambiguous => {
                self.render_unresolved_link("ambiguous-link", title.unwrap_or(target), target);
            }
        }
    }

    fn render_note_link(&mut self, target: &str) {
        self.render_note_link_with_title(target, None);
    }

    fn render_link(&mut self, title: &str, target: &str) {
        if self.context.project.is_some()
            && let Some(note_target) = maki::note_link_target_for_href(target)
        {
            self.render_note_link_with_title(&note_target, Some(title));
            return;
        }

        self.render_anchor(target, title);
    }

    fn render_inline(&mut self, inline: &Inline<'_>) {
        match inline {
            Inline::NoteLink { target } => self.render_note_link(target),
            Inline::Link { title, target } => self.render_link(title, target),
            Inline::SoftBreak => self.html.push(' '),
            Inline::Text(text) => self.escape_html_into(text),
            Inline::Code(text) => {
                self.html.push_str("<code>");
                self.escape_html_into(text);
                self.html.push_str("</code>");
            }
        }
    }
    fn render_inlines(&mut self, inlines: &[Inline<'_>]) {
        for inline in inlines {
            self.render_inline(inline);
        }
    }

    fn escape_html_attr_into(&mut self, input: &str) {
        self.escape_html_into(input);
    }

    fn render_code(&mut self, lines: &[&str], lang: Option<&str>) {
        self.html.push_str("<pre><code");
        if let Some(lang) = lang {
            self.html.push_str(" class=\"language-");
            self.escape_html_attr_into(lang);
            self.html.push('"');
        }
        self.html.push('>');
        self.render_raw_lines(lines);
        self.html.push_str("</code></pre>");
    }

    fn render_raw_lines(&mut self, lines: &[&str]) {
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                self.html.push('\n');
            }
            self.escape_html_into(line);
        }
    }

    fn render_pre(&mut self, lines: &[&str]) {
        self.html.push_str("<pre>");
        self.render_raw_lines(lines);
        self.html.push_str("</pre>");
    }

    fn render_quote(&mut self, lines: &[&str]) {
        let source = lines.join("\n");
        let parsed = parser::parse(&source);

        self.html.push_str("<blockquote>");
        for block in &parsed.document.blocks {
            self.render_block(&block.kind);
        }
        self.html.push_str("</blockquote>");
    }

    fn render_unknown_container(&mut self, kind: &str, args: &[&str], lines: &[&str]) {
        self.html
            .push_str("<pre class=\"maki-container maki-container-unknown\" data-kind=\"");
        self.escape_html_attr_into(kind);
        self.html.push('"');

        if !args.is_empty() {
            self.html.push_str(" data-args=\"");
            self.escape_html_attr_into(&args.join(" "));
            self.html.push('"');
        }

        self.html.push_str("><code>");
        self.escape_html_into(kind);
        for arg in args {
            self.html.push(' ');
            self.escape_html_into(arg);
        }
        if !lines.is_empty() {
            self.html.push('\n');
            self.render_raw_lines(lines);
        }
        self.html.push_str("</code></pre>");
    }

    fn render_container(&mut self, kind: &str, args: &[&str], lines: &[&str]) {
        match kind {
            "code" => self.render_code(lines, args.first().copied()),
            "pre" | "text" => self.render_pre(lines),
            "quote" => self.render_quote(lines),
            _ => self.render_unknown_container(kind, args, lines),
        }
    }

    fn render_block(&mut self, block: &BlockKind<'_>) {
        match block {
            BlockKind::Paragraph { body } => {
                self.html.push_str("<p>");
                self.render_inlines(body);
                self.html.push_str("</p>");
            }
            BlockKind::Code { lines, lang } => self.render_code(lines, *lang),
            BlockKind::Heading { level, body } => {
                // 문서의 title이 h1이 될 거라서 하나씩 올려줌
                self.render_heading_with_inlines(level + 1, body);
            }
            BlockKind::List { items } => self.render_list(items),
            BlockKind::Quote { lines } => self.render_quote(lines),
            BlockKind::Container { kind, args, lines } => self.render_container(kind, args, lines),
        }
    }

    fn render_list(&mut self, items: &[ListItem<'_>]) {
        let tag = match items.first().map(|item| item.kind) {
            Some(ListKind::Ordered) => "ol",
            Some(ListKind::Unordered) | None => "ul",
        };

        self.html.push('<');
        self.html.push_str(tag);
        self.html.push('>');
        for item in items {
            self.html.push_str("<li>");
            self.render_inlines(&item.body);
            if !item.children.is_empty() {
                for block in &item.children {
                    self.render_block(&block.kind);
                }
            }
            self.html.push_str("</li>");
        }
        self.html.push_str("</");
        self.html.push_str(tag);
        self.html.push('>');
    }

    fn render_heading(&mut self, level: usize, body: &str) {
        let tag = self.begin_heading(level, body);
        self.escape_html_into(body);
        self.end_heading(tag);
    }

    fn render_heading_with_inlines(&mut self, level: usize, body: &str) {
        let inlines = parser::parse_inline(body);
        let tag = self.begin_heading(level, body);

        self.render_inlines(&inlines);
        self.end_heading(tag);
    }

    fn begin_heading(&mut self, level: usize, body: &str) -> HeadingTag {
        if (1..=6).contains(&level) {
            self.html.push_str("<h");
            self.html.push_str(&level.to_string());
            self.html.push_str(" id=\"");
            self.escape_html_into(body);
            self.html.push('"');
            self.html.push('>');
            HeadingTag::Native(level)
        } else {
            self.html.push_str("<div role=\"heading\" aria-level=\"");
            self.html.push_str(&level.to_string());
            self.html.push_str("\" id=\"");
            self.escape_html_into(body);
            self.html.push_str("\">");
            HeadingTag::Aria
        }
    }

    fn end_heading(&mut self, tag: HeadingTag) {
        match tag {
            HeadingTag::Native(level) => {
                self.html.push_str("</h");
                self.html.push_str(&level.to_string());
                self.html.push('>');
            }
            HeadingTag::Aria => {
                self.html.push_str("</div>");
            }
        }
    }

    fn render(&mut self, document: &Document<'a>) -> String {
        let title = document.title();
        self.begin_html(title);
        self.render_navigation();

        if let Some(title) = title {
            self.render_heading(1, title);
        }
        for block in &document.blocks {
            self.render_block(&block.kind);
        }

        self.html.push_str("</body></html>");
        self.html.clone()
    }

    fn new_with_context(context: RenderContext<'a>) -> Self {
        Self {
            html: "".to_string(),
            context,
        }
    }

    fn new_with_asset_mode(asset_mode: AssetMode) -> Self {
        Self::new_with_context(RenderContext::default().with_asset_mode(asset_mode))
    }

    fn escape_html_into(&mut self, input: &str) {
        for ch in input.chars() {
            match ch {
                '&' => self.html.push_str("&amp;"),
                '<' => self.html.push_str("&lt;"),
                '>' => self.html.push_str("&gt;"),
                '"' => self.html.push_str("&quot;"),
                '\'' => self.html.push_str("&#39;"),
                _ => self.html.push(ch),
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RenderContext<'a> {
    project: Option<ProjectRenderContext<'a>>,
    asset_mode: AssetMode,
    project_navigation: bool,
}

impl<'a> RenderContext<'a> {
    pub(crate) fn project(
        resolve_note_link: NoteLinkResolver<'a>,
        get_note: NoteInfoGetter<'a>,
    ) -> Self {
        Self {
            project: Some(ProjectRenderContext {
                resolve_note_link,
                get_note,
            }),
            asset_mode: AssetMode::Inline,
            project_navigation: true,
        }
    }

    pub(crate) fn with_asset_mode(mut self, asset_mode: AssetMode) -> Self {
        self.asset_mode = asset_mode;
        self
    }

    pub(crate) fn with_project_navigation(mut self) -> Self {
        self.project_navigation = true;
        self
    }
}

struct ProjectRenderContext<'a> {
    resolve_note_link: NoteLinkResolver<'a>,
    get_note: NoteInfoGetter<'a>,
}

type NoteLinkResolver<'a> = &'a dyn Fn(&str) -> NoteLinkResolution;
type NoteInfoGetter<'a> = &'a dyn Fn(&NoteRef) -> Option<NoteInfo>;

pub(crate) fn render_document_with_context(
    document: &Document<'_>,
    context: RenderContext<'_>,
) -> String {
    let mut renderer = Renderer::new_with_context(context);

    renderer.render(document)
}

pub(crate) fn render_document(document: &Document<'_>) -> String {
    render_document_with_context(document, RenderContext::default())
}

pub(crate) fn render_maki_source_with_context(source: &str, context: RenderContext<'_>) -> String {
    let parsed = parser::parse(source);

    render_document_with_context(&parsed.document, context)
}

pub(crate) fn render_search_page(
    query: &str,
    results: &[SearchEntry],
    total_entries: usize,
    asset_mode: AssetMode,
) -> String {
    let mut renderer = Renderer::new_with_asset_mode(asset_mode);
    renderer.begin_project_page("Search");
    renderer.html.push_str("<main class=\"maki-search-page\">");
    renderer.html.push_str("<p class=\"maki-search-summary\">");
    if query.trim().is_empty() {
        renderer
            .html
            .push_str(&format!("Showing {total_entries} titles."));
    } else {
        renderer
            .html
            .push_str(&format!("{} matches for ", results.len()));
        renderer.html.push_str("<code>");
        renderer.escape_html_into(query);
        renderer.html.push_str("</code>.");
    }
    renderer.html.push_str("</p>");

    if results.is_empty() {
        renderer
            .html
            .push_str("<p class=\"maki-search-empty\">No matching titles.</p>");
    } else {
        renderer
            .html
            .push_str("<ul class=\"maki-search-page-results\">");
        for entry in results {
            renderer.html.push_str("<li><a href=\"");
            renderer.escape_html_attr_into(entry.path());
            renderer.html.push_str("\">");
            renderer.escape_html_into(entry.title());
            renderer.html.push_str("</a><span>");
            renderer.escape_html_into(entry.source_path());
            renderer.html.push_str("</span></li>");
        }
        renderer.html.push_str("</ul>");
    }

    renderer.html.push_str("</main></body></html>");
    renderer.html
}

pub(crate) fn render_not_found_page(path: &str, asset_mode: AssetMode) -> String {
    let mut renderer = Renderer::new_with_asset_mode(asset_mode);
    renderer.begin_project_page("Not Found");
    renderer
        .html
        .push_str("<main class=\"maki-not-found-page\">");
    renderer
        .html
        .push_str("<p class=\"maki-not-found-summary\">No Maki note is available at <code>");
    renderer.escape_html_into(path);
    renderer.html.push_str("</code>.</p>");
    renderer.html.push_str("<nav class=\"maki-not-found-actions\" aria-label=\"Not found actions\"><a href=\"/\">Home</a><a href=\"/@/\">Diagnostics</a><a href=\"/.maki/search\">Search</a></nav>");
    renderer.html.push_str("</main></body></html>");
    renderer.html
}

pub(crate) fn render_diagnostics_page(
    diagnostics: &[ProjectDiagnostic],
    total_notes: usize,
    asset_mode: AssetMode,
) -> String {
    let source = diagnostics_page_source(diagnostics, total_notes);

    render_maki_source_with_context(
        &source,
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_project_navigation(),
    )
}

fn diagnostics_page_source(diagnostics: &[ProjectDiagnostic], total_notes: usize) -> String {
    let summary = ProjectDiagnosticSummary::from_diagnostics(diagnostics);
    let mut source = String::from("--^ title: Diagnostics\n\n");

    source.push_str(&format!(
        "{} issue(s) across {total_notes} note(s): {} broken link(s), {} ambiguous link(s), {} broken external link(s), {} parser warning(s), {} read failure(s).",
        summary.total(),
        summary.broken_links(),
        summary.ambiguous_links(),
        summary.broken_external_links(),
        summary.parse_warnings(),
        summary.read_failures()
    ));
    source.push_str("\n\n");

    if diagnostics.is_empty() {
        source.push_str("No diagnostics.\n");
        return source;
    }

    let mut by_source: BTreeMap<PathBuf, Vec<&ProjectDiagnostic>> = BTreeMap::new();
    for diagnostic in diagnostics {
        by_source
            .entry(diagnostic.source_path().to_path_buf())
            .or_default()
            .push(diagnostic);
    }

    for (source_path, diagnostics) in by_source {
        let source_href = format!("/{}", source_path.with_extension("").display());
        source.push_str("== [");
        push_maki_single_line(&mut source, &source_path.display().to_string());
        source.push_str("](");
        push_maki_single_line(&mut source, &source_href);
        source.push_str(")\n\n");

        for diagnostic in diagnostics {
            source.push_str("- ");
            push_diagnostic_item(&mut source, diagnostic);
            source.push('\n');
        }
        source.push('\n');
    }

    source
}

fn push_diagnostic_item(source: &mut String, diagnostic: &ProjectDiagnostic) {
    source.push_str(diagnostic.kind().label());
    source.push_str(": ");
    if let Some(line) = diagnostic.line() {
        source.push_str("line ");
        source.push_str(&line.to_string());
        source.push_str(": ");
    }

    match diagnostic.kind() {
        ProjectDiagnosticKind::ParseWarning { message } => {
            push_maki_single_line(source, message);
        }
        ProjectDiagnosticKind::BrokenLink { target } => {
            push_maki_single_line(source, target);
        }
        ProjectDiagnosticKind::AmbiguousLink { target } => {
            push_maki_single_line(source, target);
        }
        ProjectDiagnosticKind::BrokenExternalLink { target, reason } => {
            push_maki_single_line(source, target);
            source.push_str(" (");
            push_maki_single_line(source, reason);
            source.push(')');
        }
        ProjectDiagnosticKind::ReadFailed => {
            source.push_str("failed to read note");
        }
    }
}

fn push_maki_single_line(source: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '\r' | '\n' => source.push(' '),
            _ => source.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn test_render_document() {
        let parsed = parser::parse(
            r#"--^ title: Maki

= Heading

hello <maki> & friends

--v lang: html
: <main>
: </main>

- one
- two

1. first
2. second"#,
        );

        let html = render_document(&parsed.document);

        assert!(
            html.contains(
                "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
            )
        );
        assert!(html.contains("<title>Maki</title>"));
        assert!(html.contains("<h2"));
        assert!(html.contains("<p>hello &lt;maki&gt; &amp; friends</p>"));
        assert!(html.contains(
            "<pre><code class=\"language-html\">&lt;main&gt;\n&lt;/main&gt;</code></pre>"
        ));
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
        assert!(html.contains("<ol><li>first</li><li>second</li></ol>"));
    }

    #[test]
    fn test_render_heading_supports_inline_links() {
        let parsed = parser::parse(
            r#"--^ title: Diagnostics

== [home.maki](/home)"#,
        );

        let html = render_document(&parsed.document);

        assert!(
            html.contains("<h3 id=\"[home.maki](/home)\"><a href=\"/home\">home.maki</a></h3>")
        );
    }

    #[test]
    fn render_ordered_list_with_child_paragraph() {
        let parsed = parser::parse(
            r#"1. Glider 활용 증진

   현재 Glider의 CloudData를 더 넓게 활용합니다."#,
        );

        let html = render_document(&parsed.document);

        assert!(html.contains(
            "<ol><li>Glider 활용 증진<p>현재 Glider의 CloudData를 더 넓게 활용합니다.</p></li></ol>"
        ));
    }

    #[test]
    fn project_rendering_includes_home_navigation() {
        let parsed = parser::parse("--^ title: Page\n\nbody");
        let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
        let get_note_info = |_note_ref: &NoteRef| None;

        let html = render_document_with_context(
            &parsed.document,
            RenderContext::project(&resolve_note_link, &get_note_info),
        );

        assert!(html.contains(&format!(
            "{PROJECT_NAVIGATION_HTML}<script>{SEARCH_SCRIPT}</script><script>{TOC_SCRIPT}</script><h1"
        )));
    }

    #[test]
    fn project_rendering_can_use_external_assets() {
        let parsed = parser::parse("--^ title: Page\n\nbody");
        let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
        let get_note_info = |_note_ref: &NoteRef| None;

        let html = render_document_with_context(
            &parsed.document,
            RenderContext::project(&resolve_note_link, &get_note_info)
                .with_asset_mode(AssetMode::External),
        );

        assert!(html.contains(&format!(
            "<link rel=\"stylesheet\" href=\"{CSS_ASSET_PATH}\">"
        )));
        assert!(html.contains(&format!(
            "<script src=\"{SEARCH_SCRIPT_ASSET_PATH}\"></script>"
        )));
        assert!(html.contains(&format!(
            "<script src=\"{TOC_SCRIPT_ASSET_PATH}\"></script>"
        )));
        assert!(!html.contains("<style>:root"));
        assert!(!html.contains(SEARCH_SCRIPT));
        assert!(!html.contains(TOC_SCRIPT));
    }

    #[test]
    fn test_render_builtin_containers() {
        let parsed = parser::parse(
            r#"--- code rust
fn main() {}
---

--- text
line <one>
line two
---

--- quote
= Quoted

quote body
---"#,
        );

        let html = render_document(&parsed.document);

        assert!(html.contains("<pre><code class=\"language-rust\">fn main() {}</code></pre>"));
        assert!(html.contains("<pre>line &lt;one&gt;\nline two</pre>"));
        assert!(
            html.contains(
                "<blockquote><h2 id=\"Quoted\">Quoted</h2><p>quote body</p></blockquote>"
            )
        );
    }

    #[test]
    fn quote_line_renders_inner_maki_blocks() {
        let parsed = parser::parse(
            r#"> = Quoted
>
> Body with `code`
> - item
> > nested"#,
        );

        let html = render_document(&parsed.document);
        let expected = "<blockquote><h2 id=\"Quoted\">Quoted</h2><p>Body with <code>code</code></p><ul><li>item</li></ul><blockquote><p>nested</p></blockquote></blockquote>";

        assert!(html.contains(expected));
    }

    #[test]
    fn nested_unordered_list() {
        let source = r#"- first
  - second
  - second-sibling
    - third
    - third-sibling
  - fourth but second depth

- another list"#;

        let parsed = parser::parse(source);
        let html = render_document(&parsed.document);

        assert!(html.contains(
            "<ul><li>first<ul><li>second</li><li>second-sibling<ul><li>third</li><li>third-sibling</li></ul></li><li>fourth but second depth</li></ul></li></ul><ul><li>another list</li></ul>"
        ));
    }

    #[test]
    fn test_render_ordered_list() {
        let parsed = parser::parse(
            r#"1. 블록에 property를 붙일 수 있음
2. 쿼리를 통해 검색할 수 있음
3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음"#,
        );

        let html = render_document(&parsed.document);

        assert!(html.contains("<ol><li>블록에 property를 붙일 수 있음</li><li>쿼리를 통해 검색할 수 있음</li><li>컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음</li></ol>"));
    }
}
