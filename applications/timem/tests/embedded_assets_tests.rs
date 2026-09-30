//! Build outputs must survive removal of unrelated temporary worktrees.
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/embedded_web_assets.rs"));

#[test]
fn asset_table_uses_compile_time_roots_not_build_time_absolute_paths() {
    let generated = include_str!(concat!(env!("OUT_DIR"), "/embedded_web_assets.rs"));
    let mut includes = 0;
    for line in generated
        .lines()
        .filter(|line| line.contains("include_bytes!"))
    {
        assert!(
            line.contains("concat!(env!(\"CARGO_MANIFEST_DIR\")")
                || line.contains("concat!(env!(\"OUT_DIR\")"),
            "asset reference must use the current compilation roots: {line}"
        );
        includes += 1;
    }
    assert!(includes > 0);
    assert!(generated.contains("/web-gzip/"));
}

#[test]
fn embedded_index_and_gzip_match_current_dist() {
    let expected = include_bytes!("../../../interfaces/web/dist/index.html");
    assert_eq!(embedded_web_asset("/index.html").unwrap(), expected);
    let compressed = embedded_web_asset_gzip("/index.html").unwrap();
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(compressed)
        .read_to_end(&mut decoded)
        .expect("valid gzip asset");
    assert_eq!(decoded, expected);
    assert!(embedded_web_asset("/missing-asset").is_none());
    assert!(embedded_web_asset_gzip("/missing-asset").is_none());
}
