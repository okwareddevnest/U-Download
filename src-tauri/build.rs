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
    
    // Determine the platform-specific binary directory
    let platform = get_platform_dir();
    let binaries_src = PathBuf::from("binaries").join(platform);
    
    // Ensure binaries exist in the source location
    if !binaries_src.exists() {
        panic!(
            "Binaries directory not found: {}. Please ensure platform-specific binaries are present.",
            binaries_src.display()
        );
    }
    
    // Copy binaries to the target directory for development builds
    // This ensures they're available when running `cargo run` or `npm run tauri:dev`
    let target_binaries = target_dir.join("binaries").join(platform);
    
    if let Err(e) = std::fs::create_dir_all(&target_binaries) {
        eprintln!("Warning: Failed to create target binaries directory: {}", e);
    } else {
        // Copy each binary
        for binary in &["yt-dlp", "aria2c", "ffmpeg"] {
            let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
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
    }
    
    // Tell Cargo to rerun this build script if the binaries change
    println!("cargo:rerun-if-changed=binaries");
    
    tauri_build::build()
}

fn get_platform_dir() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "windows-x64";
    
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x64";
    
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-arm64";
    
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x64";
    
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-arm64";
    
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    return "android-arm64";
    
    #[cfg(all(target_os = "android", target_arch = "arm"))]
    return "android-arm";
    
    #[cfg(all(target_os = "android", target_arch = "x86"))]
    return "android-x86";
    
    #[cfg(all(target_os = "android", target_arch = "x86_64"))]
    return "android-x64";
    
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "android", target_arch = "aarch64"),
        all(target_os = "android", target_arch = "arm"),
        all(target_os = "android", target_arch = "x86"),
        all(target_os = "android", target_arch = "x86_64"),
    )))]
    return "unknown";
}
