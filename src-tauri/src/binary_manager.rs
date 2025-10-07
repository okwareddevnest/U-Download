use std::path::{Path, PathBuf};
use tauri::{path::BaseDirectory, AppHandle, Manager, Runtime};

#[derive(Debug, Clone)]
pub struct BinaryPaths {
    pub dir: PathBuf,
    pub yt_dlp: PathBuf,
    pub aria2c: PathBuf,
    pub ffmpeg: PathBuf,
}

fn platform_dir() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    { return "windows-x64"; }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    { return "linux-x64"; }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    { return "linux-arm64"; }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    { return "macos-x64"; }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { return "macos-arm64"; }

    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    { return "android-arm64"; }

    #[cfg(all(target_os = "android", target_arch = "arm"))]
    { return "android-arm"; }

    #[cfg(all(target_os = "android", target_arch = "x86"))]
    { return "android-x86"; }

    #[cfg(all(target_os = "android", target_arch = "x86_64"))]
    { return "android-x64"; }

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
    { return "unknown"; }
}

fn exe_name(base: &str) -> String {
    #[cfg(target_os = "windows")]
    { format!("{}.exe", base) }
    #[cfg(not(target_os = "windows"))]
    { base.to_string() }
}

/// Try to resolve binaries from the application resource directory (production builds)
fn try_resolve_in_resources<R: Runtime>(
    app: &AppHandle<R>,
    base_rel: &Path,
    y_name: &str,
    a_name: &str,
    f_name: &str,
) -> Option<BinaryPaths> {
    // Method 1: Direct path to binaries/platform
    if let Ok(resource_dir) = app.path().resolve(base_rel, BaseDirectory::Resource) {
        let yt = resource_dir.join(y_name);
        let ar = resource_dir.join(a_name);
        let ff = resource_dir.join(f_name);
        
        eprintln!("Checking resource path: {}", resource_dir.display());
        eprintln!("  yt-dlp: {} (exists: {})", yt.display(), yt.exists());
        eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
        eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
        
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = resource_dir.canonicalize().unwrap_or(resource_dir);
            eprintln!("✅ Found binaries in resource directory: {}", dir.display());
            return Some(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }
    
    // Method 2: From binaries root, then platform subdirectory
    if let Ok(binaries_root) = app.path().resolve("binaries", BaseDirectory::Resource) {
        let platform_dir = binaries_root.join(base_rel.file_name()?);
        let yt = platform_dir.join(y_name);
        let ar = platform_dir.join(a_name);
        let ff = platform_dir.join(f_name);
        
        eprintln!("Checking binaries root path: {}", platform_dir.display());
        eprintln!("  yt-dlp: {} (exists: {})", yt.display(), yt.exists());
        eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
        eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
        
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = platform_dir.canonicalize().unwrap_or(platform_dir);
            eprintln!("✅ Found binaries in binaries root: {}", dir.display());
            return Some(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }
    
    None
}

/// Try to resolve binaries near the executable (for various installation methods)
fn try_resolve_near_executable(
    y_rel: &Path,
    a_rel: &Path,
    f_rel: &Path,
) -> Option<BinaryPaths> {
    let mut bases: Vec<PathBuf> = Vec::new();
    
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            eprintln!("Executable directory: {}", dir.display());
            
            // Direct paths
            bases.push(dir.to_path_buf());
            bases.push(dir.join(".."));
            
            // Resource paths
            bases.push(dir.join("resources"));
            bases.push(dir.join("..").join("resources"));
            bases.push(dir.join("..").join("Resources"));
            bases.push(dir.join("..").join("..").join("Resources"));
            
            // Platform-specific bundle paths
            #[cfg(target_os = "linux")]
            {
                // AppImage and deb/rpm package paths
                bases.push(dir.join("../lib/U-Download"));
                bases.push(dir.join("../../lib/U-Download"));
                bases.push(dir.join("../../../lib/U-Download"));
                bases.push(dir.join("../lib/udownload"));
                bases.push(dir.join("../../lib/udownload"));
                bases.push(dir.join("../../../lib/udownload"));
            }
            
            #[cfg(target_os = "macos")]
            {
                // macOS app bundle paths
                bases.push(dir.join("../Resources"));
                bases.push(dir.join("../../Resources"));
                bases.push(dir.join("../Frameworks/U-Download.app/Contents/Resources"));
            }
            
            #[cfg(target_os = "windows")]
            {
                // Windows installer paths
                bases.push(dir.join("resources"));
                bases.push(dir.join("../resources"));
                bases.push(dir.join("../../resources"));
            }
        }
    }
    
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    
    for base in bases {
        let yt = base.join(y_rel);
        let ar = base.join(a_rel);
        let ff = base.join(f_rel);
        
        eprintln!("Checking near executable path: {}", base.display());
        eprintln!("  yt-dlp: {} (exists: {})", yt.display(), yt.exists());
        eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
        eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
        
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = yt.parent().unwrap_or(Path::new(".")).to_path_buf();
            eprintln!("✅ Found binaries near executable: {}", dir.display());
            return Some(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }
    None
}

/// Try to resolve binaries from target directory (development builds)
fn try_resolve_target_dir(
    y_rel: &Path,
    a_rel: &Path,
    f_rel: &Path,
) -> Option<BinaryPaths> {
    // Check if we're running from the target directory (cargo run, npm run tauri:dev)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // For development builds, binaries are copied to target/debug or target/release
            let target_binaries_dir = exe_dir.join(y_rel.parent()?);
            let yt = target_binaries_dir.join(y_rel.file_name()?);
            let ar = target_binaries_dir.join(a_rel.file_name()?);
            let ff = target_binaries_dir.join(f_rel.file_name()?);
            
            eprintln!("Checking target directory: {}", target_binaries_dir.display());
            eprintln!("  yt-dlp: {} (exists: {})", yt.display(), yt.exists());
            eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
            eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
            
            if yt.exists() && ar.exists() && ff.exists() {
                eprintln!("✅ Found binaries in target directory: {}", target_binaries_dir.display());
                return Some(BinaryPaths {
                    dir: target_binaries_dir,
                    yt_dlp: yt,
                    aria2c: ar,
                    ffmpeg: ff,
                });
            }
        }
    }
    None
}

/// Try to resolve binaries in development mode
fn try_resolve_dev_paths(
    y_rel: &Path,
    a_rel: &Path,
    f_rel: &Path,
) -> Option<BinaryPaths> {
    // Method 1: Direct path from project root
    let direct_path = PathBuf::from("src-tauri").join(y_rel);
    
    eprintln!("Checking dev path: {}", direct_path.display());
    
    if direct_path.exists() {
        if let Some(parent) = direct_path.parent() {
            let ar = parent.join(a_rel.file_name()?);
            let ff = parent.join(f_rel.file_name()?);
            
            eprintln!("  yt-dlp: {} (exists: {})", direct_path.display(), direct_path.exists());
            eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
            eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
            
            if ar.exists() && ff.exists() {
                eprintln!("✅ Found binaries in dev mode: {}", parent.display());
                return Some(BinaryPaths {
                    dir: parent.to_path_buf(),
                    yt_dlp: direct_path,
                    aria2c: ar,
                    ffmpeg: ff,
                });
            }
        }
    }
    
    // Method 2: Absolute path from current working directory
    if let Ok(cwd) = std::env::current_dir() {
        let abs_path = cwd.join("src-tauri").join(y_rel);
        if abs_path.exists() {
            if let Some(parent) = abs_path.parent() {
                let ar = parent.join(a_rel.file_name()?);
                let ff = parent.join(f_rel.file_name()?);
                
                eprintln!("Checking absolute dev path: {}", abs_path.display());
                eprintln!("  yt-dlp: {} (exists: {})", abs_path.display(), abs_path.exists());
                eprintln!("  aria2c: {} (exists: {})", ar.display(), ar.exists());
                eprintln!("  ffmpeg: {} (exists: {})", ff.display(), ff.exists());
                
                if ar.exists() && ff.exists() {
                    eprintln!("✅ Found binaries in absolute dev path: {}", parent.display());
                    return Some(BinaryPaths {
                        dir: parent.to_path_buf(),
                        yt_dlp: abs_path,
                        aria2c: ar,
                        ffmpeg: ff,
                    });
                }
            }
        }
    }
    
    None
}

/// Enhanced binary resolution with comprehensive fallback system
pub fn resolve_paths<R: Runtime>(app: &AppHandle<R>) -> Result<BinaryPaths, String> {
    let plat = platform_dir();
    let y_name = exe_name("yt-dlp");
    let a_name = exe_name("aria2c");
    let f_name = exe_name("ffmpeg");

    let base_rel = PathBuf::from("binaries").join(plat);
    let y_rel = base_rel.join(&y_name);
    let a_rel = base_rel.join(&a_name);
    let f_rel = base_rel.join(&f_name);

    eprintln!("🔍 Resolving binaries for platform: {}", plat);
    eprintln!("   Looking for: {}, {}, {}", y_name, a_name, f_name);

    // Try all resolution methods in order of preference
    // 1. Target directory (development builds - highest priority for dev mode)
    if let Some(paths) = try_resolve_target_dir(&y_rel, &a_rel, &f_rel) {
        return Ok(paths);
    }

    // 2. Resources directory (production builds)
    if let Some(paths) = try_resolve_in_resources(app, &base_rel, &y_name, &a_name, &f_name) {
        return Ok(paths);
    }

    // 3. Near executable (various installation methods)
    if let Some(paths) = try_resolve_near_executable(&y_rel, &a_rel, &f_rel) {
        return Ok(paths);
    }

    // 4. Development paths (source tree)
    if let Some(paths) = try_resolve_dev_paths(&y_rel, &a_rel, &f_rel) {
        return Ok(paths);
    }

    // If we get here, we couldn't find the binaries
    // Provide detailed error information
    let mut error_details = Vec::new();
    
    // Check target directory
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let target_binaries = exe_dir.join("binaries").join(plat);
            error_details.push(format!("Target binaries directory: {} (exists: {})", 
                target_binaries.display(), target_binaries.exists()));
            
            if target_binaries.exists() {
                for (name, path) in [
                    ("yt-dlp", target_binaries.join(&y_name)),
                    ("aria2c", target_binaries.join(&a_name)),
                    ("ffmpeg", target_binaries.join(&f_name)),
                ] {
                    error_details.push(format!("  {}: {} (exists: {})", 
                        name, path.display(), path.exists()));
                }
            }
        }
    }
    
    // Check development binaries directory
    if let Ok(cwd) = std::env::current_dir() {
        let dev_binaries = cwd.join("src-tauri").join("binaries").join(plat);
        error_details.push(format!("Development binaries directory: {} (exists: {})", 
            dev_binaries.display(), dev_binaries.exists()));
        
        if dev_binaries.exists() {
            for (name, path) in [
                ("yt-dlp", dev_binaries.join(&y_name)),
                ("aria2c", dev_binaries.join(&a_name)),
                ("ffmpeg", dev_binaries.join(&f_name)),
            ] {
                error_details.push(format!("  {}: {} (exists: {})", 
                    name, path.display(), path.exists()));
            }
        }
    }
    
    // Check resource directory
    if let Ok(resource_dir) = app.path().resolve("binaries", BaseDirectory::Resource) {
        let res_binaries = resource_dir.join(plat);
        error_details.push(format!("Resource binaries directory: {} (exists: {})", 
            res_binaries.display(), res_binaries.exists()));
        
        if res_binaries.exists() {
            for (name, path) in [
                ("yt-dlp", res_binaries.join(&y_name)),
                ("aria2c", res_binaries.join(&a_name)),
                ("ffmpeg", res_binaries.join(&f_name)),
            ] {
                error_details.push(format!("  {}: {} (exists: {})", 
                    name, path.display(), path.exists()));
            }
        }
    }

    Err(format!(
        "❌ Failed to locate required binaries for platform '{}'.\n\
         Expected: {} (yt-dlp), {} (aria2c), {} (ffmpeg)\n\
         Searched in: {}\n\
         \n\
         Debug information:\n\
         {}\n\
         \n\
         Please ensure binaries are present in src-tauri/binaries/{} directory.\n\
         Run the build script to copy binaries to the target directory.",
        plat, y_name, a_name, f_name, base_rel.display(),
        error_details.join("\n"),
        plat
    ))
}

/// Ensure binaries have executable permissions on Unix systems
pub fn ensure_executable(paths: &BinaryPaths) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for (name, p) in [("yt-dlp", &paths.yt_dlp), ("aria2c", &paths.aria2c), ("ffmpeg", &paths.ffmpeg)] {
            if let Ok(meta) = std::fs::metadata(p) {
                let mut perms = meta.permissions();
                let mode = perms.mode();
                if mode & 0o111 == 0 {
                    eprintln!("⚠️  Binary {} lacks execute permissions, fixing...", name);
                    let new_mode = (mode | 0o755) & 0o7777;
                    perms.set_mode(new_mode);
                    std::fs::set_permissions(p, perms)
                        .map_err(|e| format!("Failed to set executable permissions on {}: {}", p.display(), e))?;
                    eprintln!("✅ Fixed permissions for {}", name);
                }
            } else {
                eprintln!("⚠️  Could not read metadata for {}", p.display());
            }
        }
    }
    Ok(())
}

/// Add the binary directory to PATH environment variable for a command
pub fn augment_path_env(cmd: &mut std::process::Command, dir: &Path) {
    if let Ok(cur) = std::env::var("PATH") {
        #[cfg(target_os = "windows")]
        let sep = ";";
        #[cfg(not(target_os = "windows"))]
        let sep = ":";
        let new_path = format!("{}{}{}", dir.display(), sep, cur);
        cmd.env("PATH", new_path);
        eprintln!("🔧 Added {} to PATH", dir.display());
    } else {
        cmd.env("PATH", dir);
        eprintln!("🔧 Set PATH to {}", dir.display());
    }
}
