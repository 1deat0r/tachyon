//! Document renderer for the architecture fixture. Parsing lives in this
//! module; HTML rendering lives in the sibling `render` module.

/// A parsed document: a title plus body lines.
pub struct Doc {
    pub title: String,
    pub lines: Vec<String>,
}

/// Parse `text` into a document. The first line is the title; every
/// remaining line is body content.
pub fn parse(text: &str) -> Doc {
    let mut lines = text.lines().map(str::to_string);
    let title = lines.next().unwrap_or_default();
    Doc {
        title,
        lines: lines.collect(),
    }
}

mod render;

pub use render::to_html;
