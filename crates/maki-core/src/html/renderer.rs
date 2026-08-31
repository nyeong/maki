use std::collections::{BTreeMap, BTreeSet};

use crate::{
    link_target::{DocumentSelector, InnerSelector, NoteLinkTarget},
    maki::{self, NoteLinkResolution},
    parser::{
        self, BlockKind, DateRange, DateStamp, Document, Inline, ListItem, ListKind,
        TableColumnAlignment, TableRow, TodoState,
    },
};

use super::{
    assets::{push_project_navigation, push_project_scripts, push_stylesheet},
    context::RenderContext,
    date_markup::{DATE_RANGE_SEPARATOR_HTML, date_stamp_class, date_stamp_delimiters},
};

pub(in crate::html) struct Renderer<'a> {
    html: String,
    context: RenderContext<'a>,
    inline_date_occurrence_index: usize,
    property_date_occurrence_index: usize,
    block_id_anchors: bool,
    reference_note_scope: ReferenceNoteScope,
    rendered_ids: BTreeSet<String>,
}

#[derive(Default)]
struct ReferenceNoteScope {
    indexes: BTreeMap<String, usize>,
    notes: Vec<ReferenceNote>,
}

struct ReferenceNote {
    key: String,
    id: String,
    backlinks: Vec<String>,
    body_html: String,
}

#[derive(Clone, Copy)]
enum ReferenceUsePresentation {
    Annotation,
    Footnote,
}

impl ReferenceUsePresentation {
    fn class_name(self) -> &'static str {
        match self {
            Self::Annotation => "maki-reference-annotation",
            Self::Footnote => "maki-reference-footnote",
        }
    }
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

        self.render_project_navigation();
    }

    fn render_document_navigation(&mut self) {
        let Some(navigation) = self.context.document_navigation.clone() else {
            return;
        };
        if navigation.is_empty() {
            return;
        }

        if !navigation.ancestors().is_empty() {
            self.html
                .push_str("<nav class=\"maki-document-breadcrumb\" aria-label=\"Breadcrumb\"><ol>");
            for (index, ancestor) in navigation.ancestors().iter().enumerate() {
                self.html.push_str("<li>");
                self.render_anchor(ancestor.path(), ancestor.title());
                if index + 1 < navigation.ancestors().len() {
                    self.html.push_str(
                        "<span class=\"maki-document-breadcrumb-separator\" aria-hidden=\"true\">›</span>",
                    );
                }
                self.html.push_str("</li>");
            }
            self.html.push_str("</ol></nav>");
        }
        if !navigation.children().is_empty() {
            self.html
                .push_str("<nav class=\"maki-document-navigation\" aria-label=\"Subdocuments\">");
            self.html
                .push_str("<span class=\"maki-document-navigation-label\">Subdocuments</span><ul>");
            for child in navigation.children() {
                self.html.push_str("<li>");
                self.render_anchor(child.path(), child.title());
                self.html.push_str("</li>");
            }
            self.html.push_str("</ul></nav>");
        }
    }

    fn render_project_navigation(&mut self) {
        if !self.context.site_header {
            push_project_navigation(&mut self.html, self.context.asset_mode);
            return;
        }

        let site_title = self.context.site_title.unwrap_or("Maki").to_string();
        self.html.push_str(
            "<header class=\"maki-site-header maki-site-header-search\"><a class=\"maki-site-mark\" href=\"/\" aria-label=\"Home\"><img src=\"/favicon.ico\" alt=\"\" width=\"48\" height=\"48\"></a><div class=\"maki-site-header-main\"><div class=\"maki-site-header-row\"><strong class=\"maki-site-title\"><a href=\"/\">",
        );
        self.escape_html_into(&site_title);
        self.html.push_str(
            "</a></strong><nav class=\"maki-site-links\" aria-label=\"Project indexes\"><a href=\"/@/recents\">Recents</a><a href=\"/@/sitemap\">Sitemap</a><a href=\"/@/dates\">Dates</a></nav></div><form class=\"maki-search\" action=\"/.maki/search\" method=\"get\" role=\"search\" data-maki-search><input class=\"maki-search-input\" type=\"search\" name=\"q\" placeholder=\"Search\" aria-label=\"Search project entries\" autocomplete=\"off\" spellcheck=\"false\" data-maki-search-input><div class=\"maki-search-results\" role=\"listbox\" hidden data-maki-search-results></div></form></div></header>",
        );
        push_project_scripts(&mut self.html, self.context.asset_mode);
    }

    fn begin_html(&mut self, title: Option<&str>) {
        self.html = String::from(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
        );
        push_stylesheet(&mut self.html, self.context.asset_mode);
        if let Some(title) = title {
            self.html.push_str("<title>");
            self.render_page_title(title);
            self.html.push_str("</title>");
        }
        self.html.push_str("</head><body>");
    }

    fn render_page_title(&mut self, title: &str) {
        self.escape_html_into(title);
        if let Some(site_title) = self.context.site_title {
            self.html.push_str(" | ");
            self.escape_html_into(site_title);
        }
    }

    pub(in crate::html) fn begin_project_page(&mut self, title: &str) {
        self.begin_html(Some(title));
        self.render_project_navigation();
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

    fn reserve_rendered_id(&mut self, id: &str) {
        self.rendered_ids.insert(id.to_string());
    }

    fn allocate_rendered_id(&mut self, base: &str) -> String {
        if self.rendered_ids.insert(base.to_string()) {
            return base.to_string();
        }

        for suffix in 2.. {
            let candidate = format!("{base}-{suffix}");
            if self.rendered_ids.insert(candidate.clone()) {
                return candidate;
            }
        }

        unreachable!("a free HTML id suffix always exists")
    }

    fn reserve_document_ids(
        &mut self,
        document: &Document<'_>,
        render_title: bool,
        block_id_anchors: bool,
    ) {
        if render_title && let Some(title) = document.title() {
            self.reserve_rendered_id(title);
        }
        self.reserve_block_ids(
            &document.blocks,
            block_id_anchors,
            document.reference_definitions(),
        );
    }

    fn reserve_block_ids(
        &mut self,
        blocks: &[parser::Block<'_>],
        block_id_anchors: bool,
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading { raw_body, .. } => {
                    let id = if block_id_anchors {
                        block.property("id").filter(|id| !id.is_empty())
                    } else {
                        None
                    }
                    .unwrap_or(raw_body);
                    self.reserve_rendered_id(id);
                }
                BlockKind::ReferenceDefinition { .. } => {}
                _ if block_id_anchors => {
                    if let Some(id) = block.property("id").filter(|id| !id.is_empty()) {
                        self.reserve_rendered_id(id);
                    }
                }
                _ => {}
            }

            match &block.kind {
                BlockKind::List { items } => {
                    for item in items {
                        self.reserve_block_ids(&item.children, block_id_anchors, references);
                    }
                }
                BlockKind::Quote { lines }
                    if !matches!(block.property("mode"), Some("pre" | "text")) =>
                {
                    self.reserve_quote_ids(lines, references);
                }
                BlockKind::Container { kind, lines, .. }
                    if *kind == "quote"
                        && !matches!(block.property("mode"), Some("pre" | "text")) =>
                {
                    self.reserve_quote_ids(lines, references);
                }
                _ => {}
            }
        }
    }

    fn reserve_quote_ids(&mut self, lines: &[&str], references: &parser::ReferenceDefinitions<'_>) {
        let source = lines.join("\n");
        let parsed = parser::parse_with_references(&source, references);
        self.reserve_document_ids(&parsed.document, false, false);
    }

    fn register_reference_use(&mut self, key: &str) -> (usize, String, String) {
        let note_index = if let Some(index) = self.reference_note_scope.indexes.get(key) {
            *index
        } else {
            let index = self.reference_note_scope.notes.len();
            let note_number = index + 1;
            let note_id = self.allocate_rendered_id(&format!("maki-reference-note-{note_number}"));
            self.reference_note_scope.notes.push(ReferenceNote {
                key: key.to_string(),
                id: note_id,
                backlinks: Vec::new(),
                body_html: String::new(),
            });
            self.reference_note_scope
                .indexes
                .insert(key.to_string(), index);
            index
        };
        let note_number = note_index + 1;
        let occurrence_number = self.reference_note_scope.notes[note_index].backlinks.len() + 1;
        let use_id = self.allocate_rendered_id(&format!(
            "maki-reference-use-{note_number}-{occurrence_number}"
        ));
        self.reference_note_scope.notes[note_index]
            .backlinks
            .push(use_id.clone());
        let note_id = self.reference_note_scope.notes[note_index].id.clone();

        (note_number, note_id, use_id)
    }

    fn render_reference_use(&mut self, key: &str, presentation: ReferenceUsePresentation) {
        let (note_number, note_id, use_id) = self.register_reference_use(key);

        self.html.push_str("<a class=\"maki-reference-use ");
        self.html.push_str(presentation.class_name());
        self.html.push_str("\" id=\"");
        self.html.push_str(&use_id);
        self.html.push_str("\" href=\"#");
        self.html.push_str(&note_id);
        self.html.push_str("\" role=\"doc-noteref\">");
        self.escape_html_into(key);
        self.html.push_str("<sup class=\"maki-reference-number\">");
        self.html.push_str(&note_number.to_string());
        self.html.push_str("</sup></a>");
    }

    fn render_footnote_reference(&mut self, label: &str) {
        self.render_reference_use(label, ReferenceUsePresentation::Footnote);
    }

    fn render_note_link_with_title(&mut self, target: &str, title: Option<&str>) {
        let Some(context) = &self.context.project else {
            let parsed = NoteLinkTarget::parse(target);
            if parsed.document == DocumentSelector::Current
                && let Some(InnerSelector::Id(id)) = parsed.inner
                && !id.is_empty()
            {
                self.render_anchor(&format!("#{id}"), title.unwrap_or(target));
                return;
            }
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
            NoteLinkResolution::FoundHeading { note, anchor } => {
                let href = format!("{}#{anchor}", note.web_path());
                self.render_anchor(&href, title.unwrap_or(target));
            }
            NoteLinkResolution::FoundId { note, id } => {
                let href = format!("{}#{id}", note.web_path());
                self.render_anchor(&href, title.unwrap_or(target));
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

    fn render_hyper_link(&mut self, target: &str) {
        let title = target
            .strip_prefix("https://")
            .or_else(|| target.strip_prefix("http://"))
            .unwrap_or(target);
        self.render_anchor(target, title);
    }

    fn render_date_stamp_text(&mut self, stamp: DateStamp<'_>) {
        let (open, close) = date_stamp_delimiters(stamp.kind());
        self.html.push(open);
        self.escape_html_into(stamp.body());
        self.html.push(close);
    }

    fn render_date_location(&mut self, occurrence_id: &str) {
        self.html
            .push_str("<span class=\"maki-date-location\" id=\"");
        self.escape_html_attr_into(occurrence_id);
        self.html.push_str("\" aria-hidden=\"true\"></span>");
    }

    fn render_date_stamp_link(
        &mut self,
        stamp: DateStamp<'_>,
        occurrence_id: &str,
        is_location: bool,
    ) {
        let href = maki::date_occurrence_href(stamp.target(), occurrence_id);
        self.html.push_str("<a class=\"");
        if is_location {
            self.html.push_str("maki-date-location ");
        }
        self.html.push_str(date_stamp_class(stamp.kind()));
        self.html.push('"');
        if is_location {
            self.html.push_str(" id=\"");
            self.escape_html_attr_into(occurrence_id);
            self.html.push('"');
        }
        self.html.push_str(" href=\"");
        self.escape_html_attr_into(&href);
        self.html.push_str("\">");
        self.render_date_stamp_text(stamp);
        self.html.push_str("</a>");
    }

    fn next_inline_date_occurrence_id(&mut self) -> Option<String> {
        let source_path = self.context.date_source_path?;
        self.inline_date_occurrence_index += 1;

        Some(maki::inline_date_occurrence_id(
            source_path,
            self.inline_date_occurrence_index,
        ))
    }

    fn next_property_date_occurrence_id(&mut self) -> Option<String> {
        let source_path = self.context.date_source_path?;
        self.property_date_occurrence_index += 1;

        Some(maki::property_date_occurrence_id(
            source_path,
            self.property_date_occurrence_index,
        ))
    }

    fn render_property_date_locations<'p>(
        &mut self,
        properties: impl Iterator<Item = (&'p str, &'p str)>,
    ) {
        for (_key, value) in properties {
            let inlines = parser::parse_inline(value);
            self.render_property_inline_date_locations(&inlines);
        }
    }

    fn render_property_inline_date_locations(&mut self, inlines: &[Inline<'_>]) {
        for inline in inlines {
            match inline {
                Inline::DateStamp(_) | Inline::DateRange(_) => {
                    if let Some(occurrence_id) = self.next_property_date_occurrence_id() {
                        self.render_date_location(&occurrence_id);
                    }
                }
                _ => {
                    if let Some(body) = inline.nested_inlines() {
                        self.render_property_inline_date_locations(body);
                    }
                }
            }
        }
    }

    fn render_date_stamp(&mut self, stamp: DateStamp<'_>) {
        if let Some(occurrence_id) = self.next_inline_date_occurrence_id() {
            self.render_date_stamp_link(stamp, &occurrence_id, true);
        } else {
            self.render_date_stamp_text(stamp);
        }
    }

    fn render_date_range(&mut self, range: DateRange<'_>) {
        if let Some(occurrence_id) = self.next_inline_date_occurrence_id() {
            self.render_date_stamp_link(range.start(), &occurrence_id, true);
            self.html.push_str(DATE_RANGE_SEPARATOR_HTML);
            self.render_date_stamp_link(range.end(), &occurrence_id, false);
        } else {
            self.render_date_stamp_text(range.start());
            self.html.push_str(DATE_RANGE_SEPARATOR_HTML);
            self.render_date_stamp_text(range.end());
        }
    }

    fn render_inline(&mut self, inline: &Inline<'_>) {
        match inline {
            Inline::NoteLink { target } => self.render_note_link(target),
            Inline::Link { title, target } => {
                if parser::reference_value_is_link_shaped(target) {
                    self.render_link(title, target.trim());
                } else {
                    self.render_reference_use(title, ReferenceUsePresentation::Annotation);
                }
            }
            Inline::Footnote { label } => self.render_footnote_reference(label),
            Inline::HyperLink { target } => self.render_hyper_link(target),
            Inline::DateStamp(stamp) => self.render_date_stamp(*stamp),
            Inline::DateRange(range) => self.render_date_range(*range),
            Inline::SoftBreak => self.html.push(' '),
            Inline::Text(text) => self.escape_html_into(text),
            Inline::Italic(body) => {
                self.html.push_str("<em>");
                self.render_inlines(body);
                self.html.push_str("</em>");
            }
            Inline::Strong(body) => {
                self.html.push_str("<strong>");
                self.render_inlines(body);
                self.html.push_str("</strong>");
            }
            Inline::Superscript(text) => {
                self.html.push_str("<sup>");
                self.escape_html_into(text);
                self.html.push_str("</sup>");
            }
            Inline::Subscript(text) => {
                self.html.push_str("<sub>");
                self.escape_html_into(text);
                self.html.push_str("</sub>");
            }
            Inline::Insertion(text) => {
                self.html.push_str("<ins>");
                self.escape_html_into(text);
                self.html.push_str("</ins>");
            }
            Inline::Deletion(text) => {
                self.html.push_str("<del>");
                self.escape_html_into(text);
                self.html.push_str("</del>");
            }
            Inline::Highlight(body) => {
                self.html.push_str("<mark>");
                self.render_inlines(body);
                self.html.push_str("</mark>");
            }
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

    pub(in crate::html) fn escape_html_attr_into(&mut self, input: &str) {
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

    fn render_quote(
        &mut self,
        lines: &[&str],
        mode: Option<&str>,
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        match mode {
            Some("pre") => {
                self.html.push_str("<blockquote>");
                self.render_pre(lines);
                self.html.push_str("</blockquote>");
                return;
            }
            Some("text") => {
                self.html
                    .push_str("<blockquote><div class=\"maki-quote-text\">");
                self.render_raw_lines(lines);
                self.html.push_str("</div></blockquote>");
                return;
            }
            _ => {}
        }

        let source = lines.join("\n");
        let parsed = parser::parse_with_references(&source, references);
        self.reserve_document_ids(&parsed.document, false, false);
        let outer_reference_note_scope = std::mem::take(&mut self.reference_note_scope);

        self.html.push_str("<blockquote>");
        self.render_document_date_locations(&parsed.document);
        let block_id_anchors = self.block_id_anchors;
        self.block_id_anchors = false;
        self.render_blocks(
            &parsed.document.blocks,
            parsed.document.reference_definitions(),
        );
        self.block_id_anchors = block_id_anchors;
        self.render_reference_notes(&parsed.document);
        self.html.push_str("</blockquote>");
        self.reference_note_scope = outer_reference_note_scope;
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

    fn render_container(
        &mut self,
        kind: &str,
        args: &[&str],
        lines: &[&str],
        lang: Option<&str>,
        mode: Option<&str>,
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        match kind {
            "code" => self.render_code(lines, args.first().copied().or(lang)),
            "pre" | "text" => self.render_pre(lines),
            "quote" => self.render_quote(lines, mode, references),
            _ => self.render_unknown_container(kind, args, lines),
        }
    }

    fn render_block(
        &mut self,
        block: &parser::Block<'_>,
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        self.render_property_date_locations(block.properties());
        if self.block_id_anchors
            && !matches!(
                &block.kind,
                BlockKind::Heading { .. } | BlockKind::ReferenceDefinition { .. }
            )
            && let Some(id) = block.property("id").filter(|id| !id.is_empty())
        {
            self.reserve_rendered_id(id);
            self.html
                .push_str("<span class=\"maki-block-anchor\" id=\"");
            self.escape_html_attr_into(id);
            self.html.push_str("\" aria-hidden=\"true\"></span>");
        }
        match &block.kind {
            BlockKind::Quote { lines } => {
                self.render_quote(lines, block.property("mode"), references)
            }
            BlockKind::Container { kind, args, lines } => self.render_container(
                kind,
                args,
                lines,
                block.property("lang"),
                block.property("mode"),
                references,
            ),
            BlockKind::Heading {
                level,
                body,
                raw_body,
            } => {
                let anchor = if self.block_id_anchors {
                    block.property("id").filter(|id| !id.is_empty())
                } else {
                    None
                }
                .unwrap_or(raw_body);
                self.render_heading_with_inlines(level + 1, anchor, body);
            }
            kind => self.render_block_kind(kind, references),
        }
    }

    fn render_block_kind(
        &mut self,
        block: &BlockKind<'_>,
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        match block {
            BlockKind::Paragraph { body } => {
                self.html.push_str("<p>");
                self.render_inlines(body);
                self.html.push_str("</p>");
            }
            BlockKind::Code { lines, lang } => self.render_code(lines, *lang),
            BlockKind::Heading { .. } => unreachable!("headings are rendered by render_block"),
            BlockKind::List { items } => self.render_list(items, references),
            BlockKind::Quote { .. } | BlockKind::Container { .. } => {
                unreachable!("property-aware blocks are rendered by render_block")
            }
            BlockKind::Table {
                header,
                alignments,
                rows,
            } => self.render_table(header, alignments, rows),
            BlockKind::ReferenceDefinition { .. } => {}
        }
    }

    fn table_cell_alignment(
        alignments: &[TableColumnAlignment],
        index: usize,
    ) -> TableColumnAlignment {
        alignments
            .get(index)
            .copied()
            .unwrap_or(TableColumnAlignment::Text)
    }

    fn render_table_alignment_attr(&mut self, alignment: TableColumnAlignment) {
        if alignment == TableColumnAlignment::Number {
            self.html.push_str(" class=\"maki-table-number\"");
        }
    }

    fn render_table_header(&mut self, row: &TableRow<'_>, alignments: &[TableColumnAlignment]) {
        self.html.push_str("<thead><tr>");
        for (index, cell) in row.cells.iter().enumerate() {
            self.html.push_str("<th");
            self.render_table_alignment_attr(Self::table_cell_alignment(alignments, index));
            self.html.push_str(" scope=\"col\">");
            self.render_inlines(&cell.body);
            self.html.push_str("</th>");
        }
        self.html.push_str("</tr></thead>");
    }

    fn render_table_body(&mut self, rows: &[TableRow<'_>], alignments: &[TableColumnAlignment]) {
        if rows.is_empty() {
            return;
        }

        self.html.push_str("<tbody>");
        for row in rows {
            if row.is_separator() {
                self.render_table_separator(alignments.len());
                continue;
            }

            self.html.push_str("<tr>");
            for (index, cell) in row.cells.iter().enumerate() {
                self.html.push_str("<td");
                self.render_table_alignment_attr(Self::table_cell_alignment(alignments, index));
                self.html.push('>');
                self.render_inlines(&cell.body);
                self.html.push_str("</td>");
            }
            self.html.push_str("</tr>");
        }
        self.html.push_str("</tbody>");
    }

    fn render_table_separator(&mut self, column_count: usize) {
        self.html
            .push_str("<tr class=\"maki-table-separator\" aria-hidden=\"true\"><td colspan=\"");
        self.html.push_str(&column_count.to_string());
        self.html.push_str("\"></td></tr>");
    }

    fn render_table(
        &mut self,
        header: &TableRow<'_>,
        alignments: &[TableColumnAlignment],
        rows: &[TableRow<'_>],
    ) {
        self.html.push_str("<table>");
        self.render_table_header(header, alignments);
        self.render_table_body(rows, alignments);
        self.html.push_str("</table>");
    }

    fn render_list(
        &mut self,
        items: &[ListItem<'_>],
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        let tag = match items.first().map(|item| item.kind) {
            Some(ListKind::Ordered) => "ol",
            Some(ListKind::Unordered) | None => "ul",
        };

        self.html.push('<');
        self.html.push_str(tag);
        self.html.push('>');
        for item in items {
            self.render_list_item_start(item.todo);
            self.render_inlines(&item.body);
            if !item.children.is_empty() {
                for block in &item.children {
                    self.render_block(block, references);
                }
            }
            self.html.push_str("</li>");
        }
        self.html.push_str("</");
        self.html.push_str(tag);
        self.html.push('>');
    }

    fn render_list_item_start(&mut self, state: Option<TodoState>) {
        let Some(state) = state else {
            self.html.push_str("<li>");
            return;
        };

        let (state_name, checked) = match state {
            TodoState::Todo => ("todo", false),
            TodoState::Done => ("done", true),
        };
        self.html
            .push_str("<li class=\"maki-todo-item\" data-todo-state=\"");
        self.html.push_str(state_name);
        self.html
            .push_str("\"><input class=\"maki-todo-checkbox\" type=\"checkbox\" disabled");
        if checked {
            self.html.push_str(" checked");
        }
        self.html.push_str(" aria-label=\"");
        self.html.push_str(state_name);
        self.html.push_str("\">");
    }

    fn render_heading(&mut self, level: usize, body: &str) {
        let tag = self.begin_heading(level, body);
        self.escape_html_into(body);
        self.end_heading(tag);
    }

    fn render_heading_with_inlines(&mut self, level: usize, raw_body: &str, body: &[Inline<'_>]) {
        let tag = self.begin_heading(level, raw_body);

        self.render_inlines(body);
        self.end_heading(tag);
    }

    fn render_reference_notes(&mut self, document: &Document<'_>) {
        if self.reference_note_scope.notes.is_empty() {
            return;
        }

        self.prepare_reference_notes(document);
        let title_id = self.allocate_rendered_id("maki-reference-notes-title");
        self.html
            .push_str("<section class=\"maki-reference-notes\" aria-labelledby=\"");
        self.html.push_str(&title_id);
        self.html
            .push_str("\"><h2 class=\"maki-reference-notes-title\" id=\"");
        self.html.push_str(&title_id);
        self.html.push_str("\">Notes</h2><ol>");

        for note_index in 0..self.reference_note_scope.notes.len() {
            let note = &self.reference_note_scope.notes[note_index];
            let key = note.key.clone();
            let note_id = note.id.clone();
            let body_html = note.body_html.clone();
            let backlinks = note.backlinks.clone();

            self.html.push_str("<li id=\"");
            self.html.push_str(&note_id);
            self.html.push_str("\" tabindex=\"-1\">");
            self.html.push_str(&body_html);
            self.html
                .push_str("<span class=\"maki-reference-backlinks\">");
            for (occurrence_index, backlink) in backlinks.iter().enumerate() {
                let accessible_label = format!("Back to {key} reference {}", occurrence_index + 1);
                self.html
                    .push_str("<a class=\"maki-reference-backlink\" href=\"#");
                self.html.push_str(backlink);
                self.html.push_str("\" aria-label=\"");
                self.escape_html_attr_into(&accessible_label);
                self.html.push_str("\">&#8617;</a>");
            }
            self.html.push_str("</span></li>");
        }
        self.html.push_str("</ol></section>");
    }

    fn prepare_reference_notes(&mut self, document: &Document<'_>) {
        let mut note_index = 0;
        while note_index < self.reference_note_scope.notes.len() {
            let key = self.reference_note_scope.notes[note_index].key.clone();
            let outer_html = std::mem::take(&mut self.html);

            if let Some(definition) = document.reference(&key) {
                let raw_value = definition.raw_value.trim();
                if parser::reference_value_is_link_shaped(raw_value) {
                    self.render_link(raw_value, raw_value);
                } else {
                    self.render_inlines(&definition.value);
                }
            } else {
                self.escape_html_into(&key);
            }

            let body_html = std::mem::replace(&mut self.html, outer_html);
            self.reference_note_scope.notes[note_index].body_html = body_html;
            note_index += 1;
        }
    }

    fn begin_heading(&mut self, level: usize, body: &str) -> HeadingTag {
        self.reserve_rendered_id(body);
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

    pub(in crate::html) fn render(&mut self, document: &Document<'a>) -> String {
        let title = document.title();
        self.reserve_document_ids(document, true, true);
        self.begin_html(title);
        self.render_navigation();

        self.render_document_date_locations(document);
        if let Some(title) = title {
            self.render_heading(1, title);
        }
        self.render_document_navigation();
        self.render_blocks(&document.blocks, document.reference_definitions());
        self.render_reference_notes(document);

        self.html.push_str("</body></html>");
        self.html.clone()
    }

    fn render_document_date_locations(&mut self, document: &Document<'_>) {
        self.render_property_date_locations(document.properties());
    }

    fn render_blocks(
        &mut self,
        blocks: &[parser::Block<'_>],
        references: &parser::ReferenceDefinitions<'_>,
    ) {
        for block in blocks {
            self.render_block(block, references);
        }
    }

    pub(in crate::html) fn new_with_context(context: RenderContext<'a>) -> Self {
        Self {
            html: "".to_string(),
            context,
            inline_date_occurrence_index: 0,
            property_date_occurrence_index: 0,
            block_id_anchors: true,
            reference_note_scope: ReferenceNoteScope::default(),
            rendered_ids: BTreeSet::new(),
        }
    }

    pub(in crate::html) fn push_raw(&mut self, input: &str) {
        self.html.push_str(input);
    }

    pub(in crate::html) fn into_html(self) -> String {
        self.html
    }

    pub(in crate::html) fn escape_html_into(&mut self, input: &str) {
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
