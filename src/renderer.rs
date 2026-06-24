use crate::parser::{BlockBody, Document, Inline};

fn render_html_note_link<F>(target: &str, resolver: &F) -> String
where
    F: for<'a> Fn(&NoteLinkQuery<'a>) -> NoteLinkResult,
{
    let result = resolver(&NoteLinkQuery { target });

    match result {
        NoteLinkResult::Found { href, label } => {
            format!("<a href=\"{}\">{}</a>", href, label)
        }
        NoteLinkResult::Broken => {
            format!("<span style=\"color: red;\">{}</span>", target)
        }
    }
}

fn render_html_inline<F>(content: &[Inline], resolver: &F) -> String
where
    F: for<'a> Fn(&NoteLinkQuery<'a>) -> NoteLinkResult,
{
    let mut html = String::new();

    for inline in content {
        match inline {
            Inline::Text(content) => {
                html.push_str(content);
            }
            Inline::NoteLink { target } => {
                html.push_str(&render_html_note_link(target, resolver));
            }
        }
    }

    html
}

pub(crate) struct NoteLinkQuery<'a> {
    target: &'a str,
}

impl NoteLinkQuery<'_> {
    pub(crate) fn target(&self) -> &str {
        self.target
    }
}

pub(crate) enum NoteLinkResult {
    Found { href: String, label: String },
    Broken,
}

/// Resolver:
pub(crate) fn render_html<F>(doc: &Document, resolver: F) -> String
where
    F: for<'a> Fn(&NoteLinkQuery<'a>) -> NoteLinkResult,
{
    let mut html = String::new();

    for block in doc.blocks() {
        match block.body() {
            BlockBody::Heading { level, content } => {
                html.push_str("<h");
                html.push_str(usize::from(*level).to_string().as_str());
                html.push('>');
                html.push_str(&render_html_inline(content, &resolver));
                html.push_str("</h");
                html.push_str(usize::from(*level).to_string().as_str());
                html.push('>');
            }
            BlockBody::Paragraph { content } => {
                html.push_str("<p>");
                html.push_str(&render_html_inline(content, &resolver));
                html.push_str("</p>");
            }
        }
    }

    html
}
