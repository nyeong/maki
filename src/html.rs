//! HTML renderer for parsed Maki documents.

use crate::{
    maki::{NoteLinkResolution, NoteRef},
    parser::{BlockKind, Document, Inline, ListItem},
};

const DEFAULT_CSS: &str = include_str!("../assets/maki.css");

pub(crate) struct NoteInfo {
    pub(crate) title: String,
}

struct Renderer<'a> {
    html: String,
    context: RenderContext<'a>,
}

impl<'a> Renderer<'a> {
    fn render_note_link(&mut self, target: &str) {
        let Some(context) = &self.context.project else {
            self.html.push_str("<a href=\"");
            self.escape_html_into(target);
            self.html.push_str("\">");
            self.escape_html_into(target);
            self.html.push_str("</a>");
            return;
        };
        match (context.resolve_note_link)(target) {
            NoteLinkResolution::Found(note_ref) => {
                let note_info = (context.get_note)(&note_ref).unwrap();
                self.html.push_str("<a href=\"");
                self.escape_html_into(&note_ref.web_path());
                self.html.push_str("\">");
                self.escape_html_into(&note_info.title);
                self.html.push_str("</a>");
            }
            NoteLinkResolution::Broken => {
                self.html.push_str("<span class=\"broken-link\">");
                self.escape_html_into(target);
                self.html.push_str("</span>");
            }
            NoteLinkResolution::Ambiguous => {
                self.html.push_str("<span class=\"ambiguous-link\">");
                self.escape_html_into(target);
                self.html.push_str("</span>");
            }
        }
    }

    fn render_inline(&mut self, inline: &Inline<'_>) {
        match inline {
            Inline::NoteLink { target } => self.render_note_link(target),
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

    fn render_block(&mut self, block: &BlockKind<'_>) {
        match block {
            BlockKind::Paragraph { body } => {
                self.html.push_str("<p>");
                for (index, inline) in body.iter().enumerate() {
                    if index > 0 {
                        self.html.push('\n');
                    }
                    self.render_inline(inline);
                }
                self.html.push_str("</p>");
            }
            BlockKind::Code { lines, lang } => {
                self.html.push_str("<pre><code");
                if let Some(lang) = lang {
                    self.html.push_str(" class=\"language-");
                    self.escape_html_attr_into(lang);
                    self.html.push('"');
                }
                self.html.push('>');
                for (index, line) in lines.iter().enumerate() {
                    if index > 0 {
                        self.html.push('\n');
                    }
                    self.escape_html_into(line);
                }
                self.html.push_str("</code></pre>");
            }
            BlockKind::Heading { level, body } => {
                // 문서의 title이 h1이 될 거라서 하나씩 올려줌
                self.render_heading(level + 1, body);
            }
            BlockKind::List { items } => self.render_list(items),
        }
    }

    fn render_list(&mut self, items: &[ListItem<'_>]) {
        self.html.push_str("<ul>");
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
        self.html.push_str("</ul>");
    }

    fn render_heading(&mut self, level: usize, body: &str) {
        if (1..=6).contains(&level) {
            self.html.push_str("<h");
            self.html.push_str(&level.to_string());
            self.html.push_str(" id=\"");
            self.escape_html_into(body);
            self.html.push('"');
            self.html.push('>');
            self.escape_html_into(body);
            self.html.push_str("</h");
            self.html.push_str(&level.to_string());
            self.html.push('>');
        } else {
            self.html.push_str("<div role=\"heading\" aria-level=\"");
            self.html.push_str(&level.to_string());
            self.html.push_str("\" id=\"");
            self.escape_html_into(body);
            self.html.push_str("\">");
            self.escape_html_into(body);
            self.html.push_str("</div>");
        }
    }
    fn render(&mut self, document: &Document<'a>) -> String {
        self.html = String::from("<!doctype html><html><head><meta charset=\"utf-8\">");
        let title = document.title();
        self.html.push_str("<style>");
        self.html.push_str(DEFAULT_CSS);
        self.html.push_str("</style>");
        if let Some(title) = title {
            self.html.push_str("<title>");
            self.escape_html_into(title);
            self.html.push_str("</title>");
        }
        self.html.push_str("</head><body>");

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
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn test_render_document() {
        let document = parser::parse(
            r#"--^ title: Maki

= Heading

hello <maki> & friends

--v lang: html
    <main>
    </main>

- one
- two"#,
        );

        let html = render_document(&document);

        assert!(html.contains("<title>Maki</title>"));
        assert!(html.contains("<h2"));
        assert!(html.contains("<p>hello &lt;maki&gt; &amp; friends</p>"));
        assert!(html.contains(
            "<pre><code class=\"language-html\">&lt;main&gt;\n&lt;/main&gt;</code></pre>"
        ));
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
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

        let doc = parser::parse(source);
        let html = render_document(&doc);

        assert!(html.contains(
            "<ul><li>first<ul><li>second</li><li>second-sibling<ul><li>third</li><li>third-sibling</li></ul></li><li>fourth but second depth</li></ul></li></ul><ul><li>another list</li></ul>"
        ));
    }

    #[test]
    fn test_render_tbd_as_preformatted_text() {
        let document = parser::parse(
            r#"1. 블록에 property를 붙일 수 있음
2. 쿼리를 통해 검색할 수 있음
3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음"#,
        );

        let html = render_document(&document);

        assert!(html.contains(
            "<pre><code class=\"language-maki\">1. 블록에 property를 붙일 수 있음\n2. 쿼리를 통해 검색할 수 있음\n3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음</code></pre>"
        ));
    }
}
