//! Collects the definition files of `deploy/catalog/` into the library, so that adding an
//! application to the catalog is adding a file (no code).
use std::{env, fs, path::PathBuf};

fn main() {
    collect("catalog", "catalog_files.rs");
    collect("marketplace", "marketplace_files.rs");
}

/// Writes `out_name` into `OUT_DIR`: the `*.toml` files of `deploy/<folder>/` as `FILES`.
fn collect(folder: &str, out_name: &str) {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("../../deploy")
        .join(folder);
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut out = String::from(
        "/// The definition files shipped with this build.\npub const FILES: &[(&str, &str)] = &[\n",
    );
    for file in &files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        out.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            file.canonicalize()
                .unwrap_or_else(|_| file.clone())
                .display()
                .to_string()
        ));
    }
    out.push_str("];\n");
    let target = PathBuf::from(env::var("OUT_DIR").unwrap_or_default()).join(out_name);
    let _ = fs::write(target, out);
}
