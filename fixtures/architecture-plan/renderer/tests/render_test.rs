use renderer::{parse, to_html};

#[test]
fn renders_title_and_paragraphs() {
    let doc = parse("Report\nline one & two\n<third>");
    assert_eq!(
        to_html(&doc),
        "<article><h1>Report</h1><p>line one &amp; two</p><p>&lt;third&gt;</p></article>"
    );
}

#[test]
fn empty_document_renders_empty_article() {
    let doc = parse("");
    assert_eq!(to_html(&doc), "<article><h1></h1></article>");
}
