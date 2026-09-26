//! Document renderer for the architecture fixture. Parsing lives here;
//! HTML rendering currently lives here too and belongs in the sibling
//! `render` module (see `tests/architecture.rs` for the constraint).

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

/// Render a document as a minimal HTML article.
pub fn to_html(doc: &Doc) -> String {
    let mut out = String::from("<article><h1>");
    out.push_str(&escape(&doc.title));
    out.push_str("</h1>");
    for line in &doc.lines {
        out.push_str("<p>");
        out.push_str(&escape(line));
        out.push_str("</p>");
    }
    out.push_str("</article>");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
