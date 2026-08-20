use std::path::{Path, PathBuf};

use super::context::AssetMode;

pub(in crate::html) const DEFAULT_CSS: &str = include_str!("../../../../assets/maki.css");
pub(in crate::html) const SEARCH_SCRIPT: &str = include_str!("../../../../assets/maki-search.js");
pub(in crate::html) const TOC_SCRIPT: &str = include_str!("../../../../assets/maki-toc.js");
pub const CSS_ASSET_PATH: &str = "/.maki/assets/maki.css";
pub const SEARCH_SCRIPT_ASSET_PATH: &str = "/.maki/assets/maki-search.js";
pub const TOC_SCRIPT_ASSET_PATH: &str = "/.maki/assets/maki-toc.js";
pub(in crate::html) const PROJECT_NAVIGATION_HTML: &str = r#"<header class="maki-nav">
<nav aria-label="Maki navigation">
<a class="maki-home-link" href="/">/</a>
<a class="maki-meta-link" href="/@/">@</a>
<form class="maki-search" action="/.maki/search" method="get" role="search" data-maki-search>
<input class="maki-search-input" type="search" name="q" placeholder="Search title" aria-label="Search titles" autocomplete="off" spellcheck="false" data-maki-search-input>
<div class="maki-search-results" role="listbox" hidden data-maki-search-results></div>
</form>
</nav>
</header>"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeAsset {
    request_path: &'static str,
    file_name: &'static str,
    content_type: &'static str,
    embedded: &'static str,
}

impl RuntimeAsset {
    pub fn request_path(&self) -> &'static str {
        self.request_path
    }

    pub fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub fn embedded(&self) -> &'static str {
        self.embedded
    }

    pub fn source_path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
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

pub fn runtime_assets() -> &'static [RuntimeAsset] {
    RUNTIME_ASSETS
}

pub fn runtime_asset_for_request_path(path: &str) -> Option<RuntimeAsset> {
    runtime_assets()
        .iter()
        .find(|asset| asset.request_path() == path)
        .copied()
}

pub(in crate::html) fn push_stylesheet(html: &mut String, asset_mode: AssetMode) {
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

pub(in crate::html) fn push_project_navigation(html: &mut String, asset_mode: AssetMode) {
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
