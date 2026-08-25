//! HTML renderer for parsed Maki documents.

mod assets;
mod context;
mod date_markup;
mod pages;
mod renderer;

#[cfg(test)]
mod tests;

pub use assets::{
    CSS_ASSET_PATH, RuntimeAsset, SEARCH_SCRIPT_ASSET_PATH, TOC_SCRIPT_ASSET_PATH,
    runtime_asset_for_request_path, runtime_assets,
};
pub use context::{AssetMode, NoteInfo, RenderContext};
pub use pages::{
    render_date_index_page, render_date_period_page, render_diagnostics_page,
    render_meta_index_page, render_not_found_page, render_recents_page, render_search_page,
    render_sitemap_page,
};

use crate::parser::{self, Document};

pub fn render_document_with_context(document: &Document<'_>, context: RenderContext<'_>) -> String {
    let mut renderer = renderer::Renderer::new_with_context(context);

    renderer.render(document)
}

pub fn render_document(document: &Document<'_>) -> String {
    render_document_with_context(document, RenderContext::default())
}

pub fn render_maki_source_with_context(source: &str, context: RenderContext<'_>) -> String {
    let parsed = parser::parse(source);

    render_document_with_context(&parsed.document, context)
}
