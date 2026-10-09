use std::{env, fs, path::Path};

fn main() {
    let dist_dir = Path::new("../../interfaces/web/dist");
    if !dist_dir.is_dir() {
        panic!(
            "Timem Web assets are missing. Run `pnpm --dir interfaces/web build` before building timem."
        );
    }

    let mut assets = Vec::new();
    collect_assets(dist_dir, dist_dir, &mut assets);
    assets.sort();

    let out_dir = std::path::PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let output = out_dir.join("embedded_web_assets.rs");
    let mut generated = String::from(
        "pub fn embedded_web_asset(path: &str) -> Option<&'static [u8]> {\n    match path {\n",
    );
    // Precompressed (gzip) twins for compressible text assets, served when
    // the request advertises gzip support. Binary assets (fonts, images)
    // stay identity-only.
    let mut compressed = String::from(
        "pub fn embedded_web_asset_gzip(path: &str) -> Option<&'static [u8]> {\n    match path {\n",
    );
    for asset in assets {
        let relative = asset.strip_prefix(dist_dir).expect("asset under dist");
        let url_path = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
        let source_path = format!("/../../interfaces/web/dist{}", url_path);
        generated.push_str(&format!(
            "        {url_path:?} => Some(include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {source_path:?}))),\n"
        ));
        if compressible_asset(&url_path) {
            let gz_path = write_gzip_asset(dist_dir, &asset, &out_dir);
            let gz_relative = format!(
                "/{}",
                gz_path
                    .strip_prefix(&out_dir)
                    .expect("gzip under OUT_DIR")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            compressed.push_str(&format!(
                "        {url_path:?} => Some(include_bytes!(concat!(env!(\"OUT_DIR\"), {gz_relative:?}))),\n"
            ));
        }
    }
    generated.push_str("        _ => None,\n    }\n}\n");
    compressed.push_str("        _ => None,\n    }\n}\n");
    generated.push('\n');
    generated.push_str(&compressed);
    fs::write(output, generated).expect("write generated embedded asset table");
}

fn collect_assets(root: &Path, directory: &Path, assets: &mut Vec<std::path::PathBuf>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory).expect("read web asset directory") {
        let path = entry.expect("read web asset entry").path();
        if path.is_dir() {
            collect_assets(root, &path, assets);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            assets.push(path);
        }
    }
    let _ = root;
}

fn compressible_asset(url_path: &str) -> bool {
    url_path.ends_with(".js")
        || url_path.ends_with(".css")
        || url_path.ends_with(".html")
        || url_path.ends_with(".svg")
        || url_path.ends_with(".json")
}

fn write_gzip_asset(dist_dir: &Path, asset: &Path, out_dir: &Path) -> std::path::PathBuf {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let relative_gz = asset
        .strip_prefix(dist_dir)
        .expect("asset under dist")
        .with_extension(format!(
            "{}.gz",
            asset
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
        ));
    let gz_path = out_dir.join("web-gzip").join(relative_gz);
    fs::create_dir_all(gz_path.parent().expect("gzip parent directory"))
        .expect("create gzip asset directory");
    let data = fs::read(asset).expect("read asset for gzip");
    let encoder = GzEncoder::new(
        fs::File::create(&gz_path).expect("create gzip asset file"),
        Compression::default(),
    );
    let mut encoder = encoder;
    encoder.write_all(&data).expect("write gzip asset");
    encoder.finish().expect("finish gzip asset");
    gz_path
}
