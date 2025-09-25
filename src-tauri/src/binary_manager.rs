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

    // Fallback to a generic directory name to satisfy compilation on any target.
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

fn try_dev_paths(rel: &str) -> Option<PathBuf> {
    // Probe a few likely dev locations relative to cwd and project structure
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(PathBuf::from(rel));
    candidates.push(PathBuf::from("src-tauri").join(rel));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(rel));
        candidates.push(cwd.join("src-tauri").join(rel));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(rel));
            candidates.push(dir.join("..").join(rel));
            candidates.push(dir.join("..").join("..").join(rel));
        }
    }
    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    None
}

fn candidate_base_dirs() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Executable directory
            bases.push(dir.to_path_buf());
            // One level up
            bases.push(dir.join(".."));
            // Common resources folder patterns
            bases.push(dir.join("resources"));
            bases.push(dir.join("..").join("resources"));
            // macOS .app bundle Resources
            bases.push(dir.join("..").join("Resources"));
            bases.push(dir.join("..").join("..").join("Resources"));
        }
    }
    // Also try current working directory and src-tauri
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
        bases.push(PathBuf::from("src-tauri"));
    }
    bases
}

pub fn resolve_paths<R: Runtime>(app: &AppHandle<R>) -> Result<BinaryPaths, String> {
    let plat = platform_dir();
    let y_name = exe_name("yt-dlp");
    let a_name = exe_name("aria2c");
    let f_name = exe_name("ffmpeg");

    let base_rel = PathBuf::from("binaries").join(plat);
    let y_rel = base_rel.join(&y_name);
    let a_rel = base_rel.join(&a_name);
    let f_rel = base_rel.join(&f_name);

    // 0) Prefer binaries bundled as Tauri resources (packaged builds)
    if let Ok(resource_dir) = app.path().resolve(base_rel.clone(), BaseDirectory::Resource) {
        let yt = resource_dir.join(&y_name);
        let ar = resource_dir.join(&a_name);
        let ff = resource_dir.join(&f_name);
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = resource_dir.canonicalize().unwrap_or(resource_dir.clone());
            return Ok(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }

    // 1) Look near the executable and in common resource dirs
    for base in candidate_base_dirs() {
        let yt = base.join(&y_rel);
        let ar = base.join(&a_rel);
        let ff = base.join(&f_rel);
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = yt.parent().unwrap_or(Path::new(".")).to_path_buf();
            return Ok(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }

    // 2) Try dev paths (repo layout)
    let y = try_dev_paths(&format!("src-tauri/{}", y_rel.display()))
        .or_else(|| try_dev_paths(&y_rel.display().to_string()))
        .ok_or_else(|| format!("Bundled yt-dlp not found: {}", y_rel.display()))?;
    let a = try_dev_paths(&format!("src-tauri/{}", a_rel.display()))
        .or_else(|| try_dev_paths(&a_rel.display().to_string()))
        .ok_or_else(|| format!("Bundled aria2c not found: {}", a_rel.display()))?;
    let f = try_dev_paths(&format!("src-tauri/{}", f_rel.display()))
        .or_else(|| try_dev_paths(&f_rel.display().to_string()))
        .ok_or_else(|| format!("Bundled ffmpeg not found: {}", f_rel.display()))?;
    let dir = y.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(BinaryPaths { dir, yt_dlp: y, aria2c: a, ffmpeg: f })
}

pub fn ensure_executable(paths: &BinaryPaths) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&paths.yt_dlp, &paths.aria2c, &paths.ffmpeg] {
            if let Ok(meta) = std::fs::metadata(p) {
                let mut perms = meta.permissions();
                let mode = perms.mode();
                // ensure 0o755
                if mode & 0o111 == 0 { // no exec bits
                    let new_mode = (mode | 0o755) & 0o7777;
                    perms.set_mode(new_mode);
                    std::fs::set_permissions(p, perms)
                        .map_err(|e| format!("Failed to set executable permissions on {}: {}", p.display(), e))?;
                }
            }
        }
    }
    Ok(())
}

pub fn augment_path_env(cmd: &mut std::process::Command, dir: &Path) {
    if let Ok(cur) = std::env::var("PATH") {
        #[cfg(target_os = "windows")]
        let sep = ";";
        #[cfg(not(target_os = "windows"))]
        let sep = ":";
        let new_path = format!("{}{}{}", dir.display(), sep, cur);
        cmd.env("PATH", new_path);
    } else {
        cmd.env("PATH", dir);
    }
}
