fn main() {
    use std::process::Command;
    for (name, args) in [
        ("SLOPBAR_BUILD_COMMIT", vec!["rev-parse", "HEAD"]),
        ("SLOPBAR_BUILD_TREE", vec!["rev-parse", "HEAD^{tree}"]),
    ] {
        if let Ok(out) = Command::new("git").args(args).output() {
            let value = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if out.status.success()
                && value.len() == 40
                && value.bytes().all(|b| b.is_ascii_hexdigit())
            {
                println!("cargo:rustc-env={name}={value}");
            }
        }
    }
    let clean = Command::new("git")
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ])
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.is_empty());
    println!("cargo:rustc-env=SLOPBAR_BUILD_DIRTY={}", !clean);
    // Also rebuild provenance after a commit that doesn't change application source.
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
    {
        if out.status.success() {
            let dir = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            println!("cargo:rerun-if-changed={}", dir.join("HEAD").display());
            println!("cargo:rerun-if-changed={}", dir.join("refs").display());
            println!("cargo:rerun-if-changed={}", dir.join("index").display());
        }
    }
    // Cargo can rerun rustc for source edits without rerunning this script.
    // Watch source directories and tracked files, never the checkout root: Cargo
    // recursively scans watched directories, including generated target output.
    for path in [
        "src",
        "Cargo.toml",
        "Cargo.lock",
        "tauri.conf.json",
        "build.rs",
        "../src",
        "icons",
        "../scripts",
        "../.github",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if out.status.success() {
            let root = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if let Ok(files) = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["ls-files", "-z"])
                .output()
            {
                if files.status.success() {
                    for file in files.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                        println!(
                            "cargo:rerun-if-changed={}",
                            root.join(String::from_utf8_lossy(file).as_ref()).display()
                        );
                    }
                }
            }
        }
    }
    #[cfg(feature = "desktop")]
    {
        // Tauri's dependency tells us the ACTUAL custom-protocol configuration.
        println!(
            "cargo:rustc-env=SLOPBAR_TAURI_DEV={}",
            tauri_build::is_dev()
        );
        tauri_build::build()
    }
}
