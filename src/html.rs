//! HTML renderer for parsed Maki documents.

use crate::{
    maki::{NoteLinkResolution, NoteRef, SearchEntry},
    parser::{self, BlockKind, Document, Inline, ListItem},
};

const DEFAULT_CSS: &str = include_str!("../assets/maki.css");
const PROJECT_NAVIGATION_HTML: &str = r#"<header class="maki-nav">
<nav aria-label="Maki navigation">
<a class="maki-home-link" href="/">/</a>
<form class="maki-search" action="/.maki/search" method="get" role="search" data-maki-search>
<input class="maki-search-input" type="search" name="q" placeholder="Search title" aria-label="Search titles" autocomplete="off" spellcheck="false" data-maki-search-input>
<div class="maki-search-results" role="listbox" hidden data-maki-search-results></div>
</form>
</nav>
</header>"#;
const SEARCH_SCRIPT_HTML: &str = r#"<script>(() => {
const SEARCH_INDEX_PATH = "/.maki/search-index.json";
const SEARCH_PATH = "/.maki/search";
const MAX_RESULTS = 8;
const forms = document.querySelectorAll("[data-maki-search]");
if (!forms.length) return;

let entriesPromise;
const loadEntries = () => {
  if (!entriesPromise) {
    entriesPromise = fetch(SEARCH_INDEX_PATH, { headers: { Accept: "application/json" } })
      .then(response => response.ok ? response.json() : []);
  }
  return entriesPromise;
};
const normalize = value => value.toLocaleLowerCase();
const matchRank = (entry, query) => {
  const title = normalize(entry.title || "");
  if (title === query) return [0, 0, title.length];
  if (title.startsWith(query)) return [1, 0, title.length];
  const index = title.indexOf(query);
  return index === -1 ? null : [2, index, title.length];
};
const findMatches = (entries, rawQuery) => {
  const query = normalize(rawQuery.trim());
  if (!query) return [];
  return entries
    .map(entry => ({ entry, rank: matchRank(entry, query) }))
    .filter(match => match.rank)
    .sort((left, right) => {
      for (let index = 0; index < left.rank.length; index += 1) {
        if (left.rank[index] !== right.rank[index]) return left.rank[index] - right.rank[index];
      }
      return (left.entry.title || "").localeCompare(right.entry.title || "");
    })
    .slice(0, MAX_RESULTS)
    .map(match => match.entry);
};

forms.forEach(form => {
  const input = form.querySelector("[data-maki-search-input]");
  const results = form.querySelector("[data-maki-search-results]");
  if (!input || !results) return;

  if (location.pathname === SEARCH_PATH && !input.value) {
    input.value = new URLSearchParams(location.search).get("q") || "";
  }

  let activeIndex = -1;
  let currentMatches = [];
  let requestId = 0;

  const close = () => {
    results.hidden = true;
    results.textContent = "";
    activeIndex = -1;
    currentMatches = [];
  };
  const setActive = index => {
    activeIndex = index;
    Array.from(results.children).forEach((child, childIndex) => {
      const selected = childIndex === activeIndex;
      child.classList.toggle("is-active", selected);
      child.setAttribute("aria-selected", selected ? "true" : "false");
    });
  };
  const render = matches => {
    results.textContent = "";
    currentMatches = matches;
    if (!matches.length) {
      close();
      return;
    }

    matches.forEach((entry, index) => {
      const link = document.createElement("a");
      link.className = "maki-search-result";
      link.href = entry.path;
      link.setAttribute("role", "option");
      link.setAttribute("aria-selected", "false");
      link.addEventListener("mouseenter", () => setActive(index));

      const title = document.createElement("span");
      title.className = "maki-search-result-title";
      title.textContent = entry.title;
      const source = document.createElement("span");
      source.className = "maki-search-result-source";
      source.textContent = entry.source_path;

      link.append(title, source);
      results.append(link);
    });

    results.hidden = false;
    setActive(0);
  };
  const update = async () => {
    const id = ++requestId;
    const query = input.value.trim();
    if (!query) {
      close();
      return;
    }
    const entries = await loadEntries();
    if (id !== requestId) return;
    render(findMatches(entries, query));
  };

  input.addEventListener("input", update);
  input.addEventListener("focus", () => {
    if (input.value.trim()) update();
  });
  input.addEventListener("keydown", event => {
    if (!currentMatches.length) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((activeIndex + 1) % currentMatches.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((activeIndex - 1 + currentMatches.length) % currentMatches.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      location.href = currentMatches[Math.max(activeIndex, 0)].path;
    } else if (event.key === "Escape") {
      close();
    }
  });
  form.addEventListener("submit", async event => {
    const query = input.value.trim();
    if (!query) return;
    event.preventDefault();
    const matches = findMatches(await loadEntries(), query);
    location.href = matches.length ? matches[0].path : `${SEARCH_PATH}?q=${encodeURIComponent(query)}`;
  });
  document.addEventListener("click", event => {
    if (!form.contains(event.target)) close();
  });
});
})();</script>"#;

pub(crate) struct NoteInfo {
    pub(crate) title: String,
}

fn push_project_navigation(html: &mut String) {
    html.push_str(PROJECT_NAVIGATION_HTML);
    html.push_str(SEARCH_SCRIPT_HTML);
}

struct Renderer<'a> {
    html: String,
    context: RenderContext<'a>,
}

impl<'a> Renderer<'a> {
    fn render_navigation(&mut self) {
        if self.context.project.is_none() {
            return;
        }

        push_project_navigation(&mut self.html);
    }

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
                for (index, inline) in body.iter().enumerate() {
                    if index > 0 {
                        self.html.push('\n');
                    }
                    self.render_inline(inline);
                }
                self.html.push_str("</p>");
            }
            BlockKind::Code { lines, lang } => self.render_code(lines, *lang),
            BlockKind::Heading { level, body } => {
                // 문서의 title이 h1이 될 거라서 하나씩 올려줌
                self.render_heading(level + 1, body);
            }
            BlockKind::List { items } => self.render_list(items),
            BlockKind::Container { kind, args, lines } => self.render_container(kind, args, lines),
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

pub(crate) fn render_search_page(
    query: &str,
    results: &[SearchEntry],
    total_entries: usize,
) -> String {
    let mut renderer = Renderer::new_with_context(RenderContext::default());
    renderer.html = String::from("<!doctype html><html><head><meta charset=\"utf-8\">");
    renderer.html.push_str("<style>");
    renderer.html.push_str(DEFAULT_CSS);
    renderer.html.push_str("</style>");
    renderer.html.push_str("<title>Search</title>");
    renderer.html.push_str("</head><body>");
    push_project_navigation(&mut renderer.html);
    renderer.render_heading(1, "Search");
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
- two"#,
        );

        let html = render_document(&parsed.document);

        assert!(html.contains("<title>Maki</title>"));
        assert!(html.contains("<h2"));
        assert!(html.contains("<p>hello &lt;maki&gt; &amp; friends</p>"));
        assert!(html.contains(
            "<pre><code class=\"language-html\">&lt;main&gt;\n&lt;/main&gt;</code></pre>"
        ));
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
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

        assert!(html.contains(&format!("{PROJECT_NAVIGATION_HTML}{SEARCH_SCRIPT_HTML}<h1")));
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
    fn test_render_tbd_as_preformatted_text() {
        let parsed = parser::parse(
            r#"1. 블록에 property를 붙일 수 있음
2. 쿼리를 통해 검색할 수 있음
3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음"#,
        );

        let html = render_document(&parsed.document);

        assert!(html.contains(
            "<pre><code class=\"language-maki\">1. 블록에 property를 붙일 수 있음\n2. 쿼리를 통해 검색할 수 있음\n3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음</code></pre>"
        ));
    }
}
