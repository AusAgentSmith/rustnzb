use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/src/");
    println!("cargo:rerun-if-changed=frontend/angular.json");

    // Only build Angular if the frontend directory exists
    if std::path::Path::new("frontend/package.json").exists() {
        let status = Command::new("npx")
            .args(["ng", "build", "--configuration=production"])
            .current_dir("frontend")
            .status()
            .expect("Failed to run ng build - is Node.js installed?");

        if !status.success() {
            panic!("Angular build failed");
        }
    }
}
