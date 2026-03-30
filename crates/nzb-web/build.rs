fn main() {
    // Ensure the Angular dist directory exists so rust-embed can compile.
    // The actual Angular build is handled by the workspace root build.rs
    // or CI. This just creates a placeholder if nothing exists yet.
    let dist = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../frontend/dist/frontend/browser"
    );
    let index = format!("{dist}/index.html");
    if !std::path::Path::new(&index).exists() {
        std::fs::create_dir_all(dist).ok();
        std::fs::write(
            &index,
            "<!DOCTYPE html><html><body><h1>rustnzb</h1><p>Frontend not built.</p></body></html>",
        )
        .ok();
    }
}
