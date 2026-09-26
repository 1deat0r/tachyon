//! The subsystem constraint this fixture deliberately violates: HTML
//! rendering belongs in `render.rs`, not inline in `lib.rs`.

use std::path::Path;

fn src(rel: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(Path::new(manifest).join("src").join(rel))
        .expect("fixture source readable")
}

#[test]
fn rendering_logic_lives_in_render_module() {
    let lib = src("lib.rs");
    let render = src("render.rs");
    assert!(
        lib.lines().any(|line| line.trim() == "mod render;"),
        "lib.rs must declare the render module"
    );
    assert!(
        !lib.contains("pub fn to_html"),
        "lib.rs must not define to_html; render.rs owns it"
    );
    assert!(
        render.contains("pub fn to_html"),
        "render.rs must own the to_html implementation"
    );
}
