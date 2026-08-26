use std::env;
use std::path::PathBuf;

fn main() {
    // Get the target directory where Rust builds the binary
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let target_dir = PathBuf::from(&out_dir)
        .ancestors()
        .nth(3)
        .expect("Failed to determine target directory")
        .to_path_buf();

    // Build scripts are compiled for and run on the HOST, so `cfg!`/`#[cfg]`
    // here describes the host machine, not what we are building for. When the
    // two differ (e.g. an arm64 macOS runner building x86_64-apple-darwin) that
    // resolves the wrong bundled-binaries directory. Cargo hands the build
    // script the *target* in these environment variables instead — use them.
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");

    // Determine the platform-specific binary directory
    let platform = get_platform_dir(&target_os, &target_arch);
    let binaries_src = PathBuf::from("binaries").join(platform);

    // Ensure binaries exist in the source location
    if !binaries_src.exists() {
        let triple = env::var("TARGET").unwrap_or_else(|_| "<unknown>".to_string());
        panic!(
            "Binaries directory not found: {missing}\n\
             \n\
             Resolved platform \"{platform}\" from the build target: \
             os = {target_os}, arch = {target_arch}, triple = {triple}.\n\
             (The platform follows the build *target*, not the host, so a \
             cross-compile needs that target's binaries.)\n\
             \n\
             Populate it with:  scripts/fetch-binaries.sh {platform}",
            missing = binaries_src.display(),
            platform = platform,
            target_os = target_os,
            target_arch = target_arch,
            triple = triple,
        );
    }

    // Copy binaries to the target directory for development builds
    // This ensures they're available when running `cargo run` or `npm run tauri:dev`
    let target_binaries = target_dir.join("binaries").join(platform);

    if let Err(e) = std::fs::create_dir_all(&target_binaries) {
        eprintln!("Warning: Failed to create target binaries directory: {}", e);
    } else {
        // Executable suffix, again from the target rather than the host.
        let ext = if target_os == "windows" { ".exe" } else { "" };

        // Copy each binary
        for binary in &["yt-dlp", "aria2c", "ffmpeg"] {
            let binary_name = format!("{}{}", binary, ext);

            let src = binaries_src.join(&binary_name);
            let dst = target_binaries.join(&binary_name);

            if src.exists() {
                if let Err(e) = std::fs::copy(&src, &dst) {
                    eprintln!("Warning: Failed to copy {} to target directory: {}", binary_name, e);
                } else {
                    println!("cargo:rerun-if-changed={}", src.display());

                    // Set executable permissions on Unix systems
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(metadata) = std::fs::metadata(&dst) {
                            let mut permissions = metadata.permissions();
                            permissions.set_mode(0o755);
                            let _ = std::fs::set_permissions(&dst, permissions);
                        }
                    }
                }
            } else {
                eprintln!("Warning: Binary not found: {}", src.display());
            }
        }

        // The CA bundle rides along with them. It is data, not a program, so
        // it is copied without the executable bit — but a dev build that
        // resolves its binaries out of the target directory needs it there,
        // otherwise the bundled ffmpeg has no trust store and every HTTPS
        // fetch it makes (every trimmed download among them) fails
        // certificate verification. Absent on trees that predate the fetch
        // script shipping one; resolution degrades to the host's trust store
        // there, so this is only a warning.
        let ca_src = binaries_src.join("cacert.pem");
        if ca_src.exists() {
            let ca_dst = target_binaries.join("cacert.pem");
            if let Err(e) = std::fs::copy(&ca_src, &ca_dst) {
                eprintln!("Warning: Failed to copy cacert.pem to target directory: {}", e);
            } else {
                println!("cargo:rerun-if-changed={}", ca_src.display());
            }
        } else {
            eprintln!(
                "Warning: CA bundle not found: {} — run scripts/fetch-binaries.sh",
                ca_src.display()
            );
        }
    }

    // Tell Cargo to rerun this build script if the binaries change
    println!("cargo:rerun-if-changed=binaries");

    tauri_build::build()
}

/// Map a build *target* (as reported by Cargo's `CARGO_CFG_TARGET_OS` /
/// `CARGO_CFG_TARGET_ARCH`) to the bundled-binaries directory name.
///
/// These names are also resolved at runtime by `binary_manager.rs`, so they
/// must stay exactly in sync with it.
fn get_platform_dir(target_os: &str, target_arch: &str) -> &'static str {
    match (target_os, target_arch) {
        ("windows", "x86_64") => "windows-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("android", "aarch64") => "android-arm64",
        ("android", "arm") => "android-arm",
        ("android", "x86") => "android-x86",
        ("android", "x86_64") => "android-x64",
        _ => "unknown",
    }
}
