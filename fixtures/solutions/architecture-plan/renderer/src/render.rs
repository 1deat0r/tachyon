//! Rendering half of the document subsystem: `to_html` lives here.

use super::Doc;

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
