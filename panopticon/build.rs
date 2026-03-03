use std::fs;
use std::path::Path;
use std::process::Command;

pub fn main() {
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/bun.lock");

    // Always ensure web/build directory exists for include_dir! macro
    // This is required even in dev mode because include_dir! runs at compile time
    let web_build_path = Path::new("web/build");
    if !web_build_path.exists() {
        fs::create_dir_all(web_build_path)
            .expect("Failed to create web/build directory for include_dir! macro");
    }

    // Skip web build if environment variable is set
    if std::env::var("SKIP_WEB_BUILD").is_ok() {
        println!("Skipping web build due to SKIP_WEB_BUILD environment variable");
        return;
    }

    // Skip web build in development mode (non-release builds)
    // In dev mode, frontend runs on http://localhost:5173 via Vite dev server
    // Backend runs on http://localhost:1337
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    if profile != "release" {
        println!(
            "Skipping web build in development mode (profile: {})",
            profile
        );
        println!("Frontend should be accessed at http://localhost:5173");
        println!("Backend will run on http://localhost:1337");
        return;
    }

    println!("Building frontend for release...");

    let web_dir = if Path::new("web").exists() {
        "web"
    } else {
        panic!("Could not find web directory");
    };

    // Run bun install to ensure dependencies are up to date
    println!("Running bun install...");
    let install_output = Command::new("bun")
        .args(["install"])
        .current_dir(web_dir)
        .output()
        .expect("Failed to execute bun install");

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        let stdout = String::from_utf8_lossy(&install_output.stdout);
        panic!("bun install failed:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}");
    }

    // Run bun run build in the web directory
    println!("Running bun run build...");
    let output = Command::new("bun")
        .args(["run", "build"])
        .current_dir(web_dir)
        .output()
        .expect("Failed to execute bun run build");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("bun run build failed:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}");
    }
}
