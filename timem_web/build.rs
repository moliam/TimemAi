use std::{env, fs, path::Path};

fn main() {
    let dist_dir = Path::new("../web_ui/timem-web/dist");
    if !dist_dir.is_dir() {
        panic!(
            "Timem Web assets are missing. Run `pnpm --dir web_ui/timem-web build` before building timem-web."
        );
    }

    let mut assets = Vec::new();
    collect_assets(dist_dir, dist_dir, &mut assets);
    assets.sort();

    let output = Path::new(&env::var("OUT_DIR").expect("OUT_DIR must be set"))
        .join("embedded_web_assets.rs");
    let mut generated = String::from(
        "pub fn embedded_web_asset(path: &str) -> Option<&'static [u8]> {\n    match path {\n",
    );
    for asset in assets {
        let relative = asset.strip_prefix(dist_dir).expect("asset under dist");
        let url_path = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
        let absolute = asset.canonicalize().expect("asset canonical path");
        generated.push_str(&format!(
            "        {url_path:?} => Some(include_bytes!({absolute:?})),\n"
        ));
    }
    generated.push_str("        _ => None,\n    }\n}\n");
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
