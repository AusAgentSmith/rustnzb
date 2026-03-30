use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/src/");
    println!("cargo:rerun-if-changed=frontend/angular.json");

    // Only build Angular if frontend exists AND dist is missing/stale
    if !std::path::Path::new("frontend/package.json").exists() {
        return;
    }

    // If dist already exists (e.g. CI pre-built it), skip ng build
    if std::path::Path::new("frontend/dist/frontend/browser/index.html").exists() {
        return;
    }

    // Try to run ng build; don't panic if Node.js isn't installed
    match Command::new("npx")
        .args(["ng", "build", "--configuration=production"])
        .current_dir("frontend")
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!(
                "cargo:warning=Angular build failed with exit code {:?}",
                status.code()
            );
        }
        Err(e) => {
            println!("cargo:warning=Could not run ng build (Node.js not found?): {e}");
            println!("cargo:warning=The embedded web UI will be missing. Run 'cd frontend && npx ng build' first.");
        }
    }
}
