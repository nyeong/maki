//! HTML renderer for parsed Maki documents.

use crate::parser::{BlockKind, Document, Inline};

const DEFAULT_CSS: &str = include_str!("../assets/maki.css");

pub(crate) fn render_document(document: &Document<'_>) -> String {
    let mut html = String::from("<!doctype html><html><head><meta charset=\"utf-8\">");
    let title = document.title();
    html.push_str("<style>");
    html.push_str(DEFAULT_CSS);
    html.push_str("</style>");
    if let Some(title) = title {
        html.push_str("<title>");
        escape_html_into(&mut html, title);
        html.push_str("</title>");
    }
    html.push_str("</head><body>");

    if let Some(title) = title {
        render_heading(&mut html, 1, title);
    }
    for block in &document.blocks {
        render_block(&mut html, &block.kind);
    }

    html.push_str("</body></html>");
    html
}

fn render_inline(html: &mut String, inline: &Inline<'_>) {
    match inline {
        Inline::NoteLink { target } => {
            html.push_str("<a href=\"");
            escape_html_into(html, target);
            html.push_str("\">");
            escape_html_into(html, target);
            html.push_str("</a>");
        }
        Inline::SoftBreak => html.push(' '),
        Inline::Text(text) => escape_html_into(html, text),
    }
}

fn render_inlines(html: &mut String, inlines: &[Inline<'_>]) {
    for inline in inlines {
        render_inline(html, inline);
    }
}

fn render_block(html: &mut String, block: &BlockKind<'_>) {
    match block {
        BlockKind::Paragraph { body } => {
            html.push_str("<p>");
            for (index, inline) in body.iter().enumerate() {
                if index > 0 {
                    html.push('\n');
                }
                render_inline(html, inline);
            }
            html.push_str("</p>");
        }
        BlockKind::Code { lines, lang } => {
            html.push_str("<pre><code");
            if let Some(lang) = lang {
                html.push_str(" class=\"language-");
                escape_html_attr_into(html, lang);
                html.push('"');
            }
            html.push('>');
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    html.push('\n');
                }
                escape_html_into(html, line);
            }
            html.push_str("</code></pre>");
        }
        BlockKind::Heading { level, body } => {
            // 문서의 title이 h1이 될 거라서 하나씩 올려줌
            render_heading(html, level + 1, body);
        }
        BlockKind::List { items } => {
            html.push_str("<ul>");
            for item in items {
                html.push_str("<li>");
                render_inlines(html, &item.body);
                html.push_str("</li>");
            }
            html.push_str("</ul>");
        }
    }
}

fn render_heading(html: &mut String, level: usize, body: &str) {
    if (1usize..=6usize).contains(&level) {
        html.push_str("<h");
        html.push_str(&level.to_string());
        html.push('>');
        escape_html_into(html, body);
        html.push_str("</h");
        html.push_str(&level.to_string());
        html.push('>');
    } else {
        html.push_str("<div role=\"heading\" aria-level=\"");
        html.push_str(&level.to_string());
        html.push_str("\">");
        escape_html_into(html, body);
        html.push_str("</div>");
    }
}

fn escape_html_into(output: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(ch),
        }
    }
}

fn escape_html_attr_into(output: &mut String, input: &str) {
    escape_html_into(output, input);
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
        assert!(html.contains("<h2>Heading</h2>"));
        assert!(html.contains("<p>hello &lt;maki&gt; &amp; friends</p>"));
        assert!(html.contains(
            "<pre><code class=\"language-html\">&lt;main&gt;\n&lt;/main&gt;</code></pre>"
        ));
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
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
