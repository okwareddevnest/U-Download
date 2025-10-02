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

fn try_resolve_in_resources<R: Runtime>(
    app: &AppHandle<R>,
    base_rel: &Path,
    y_name: &str,
    a_name: &str,
    f_name: &str,
) -> Option<BinaryPaths> {
    if let Ok(resource_dir) = app.path().resolve(base_rel, BaseDirectory::Resource) {
        let yt = resource_dir.join(y_name);
        let ar = resource_dir.join(a_name);
        let ff = resource_dir.join(f_name);
        
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = resource_dir.canonicalize().unwrap_or(resource_dir);
            return Some(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }
    None
}

fn try_resolve_near_executable(
    y_rel: &Path,
    a_rel: &Path,
    f_rel: &Path,
) -> Option<BinaryPaths> {
    let mut bases: Vec<PathBuf> = Vec::new();
    
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
            bases.push(dir.join(".."));
            bases.push(dir.join("resources"));
            bases.push(dir.join("..").join("resources"));
            bases.push(dir.join("..").join("Resources"));
            bases.push(dir.join("..").join("..").join("Resources"));
        }
    }
    
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    
    for base in bases {
        let yt = base.join(y_rel);
        let ar = base.join(a_rel);
        let ff = base.join(f_rel);
        
        if yt.exists() && ar.exists() && ff.exists() {
            let dir = yt.parent().unwrap_or(Path::new(".")).to_path_buf();
            return Some(BinaryPaths { dir, yt_dlp: yt, aria2c: ar, ffmpeg: ff });
        }
    }
    None
}

fn try_resolve_dev_paths(
    y_rel: &Path,
    a_rel: &Path,
    f_rel: &Path,
) -> Option<BinaryPaths> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    
    let y_src_tauri = PathBuf::from("src-tauri").join(y_rel);
    
    candidates.push(y_rel.to_path_buf());
    candidates.push(y_src_tauri.clone());
    
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(y_rel));
        candidates.push(cwd.join(&y_src_tauri));
    }
    
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(y_rel));
            candidates.push(dir.join(&y_src_tauri));
            candidates.push(dir.join("..").join(y_rel));
            candidates.push(dir.join("..").join("..").join(y_rel));
        }
    }
    
    for y_candidate in &candidates {
        if y_candidate.exists() {
            if let Some(y_parent) = y_candidate.parent() {
                let ar = y_parent.join(a_rel.file_name()?);
                let ff = y_parent.join(f_rel.file_name()?);
                
                if ar.exists() && ff.exists() {
                    return Some(BinaryPaths {
                        dir: y_parent.to_path_buf(),
                        yt_dlp: y_candidate.clone(),
                        aria2c: ar,
                        ffmpeg: ff,
                    });
                }
            }
        }
    }
    None
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

    if let Some(paths) = try_resolve_in_resources(app, &base_rel, &y_name, &a_name, &f_name) {
        return Ok(paths);
    }

    if let Some(paths) = try_resolve_near_executable(&y_rel, &a_rel, &f_rel) {
        return Ok(paths);
    }

    if let Some(paths) = try_resolve_dev_paths(&y_rel, &a_rel, &f_rel) {
        return Ok(paths);
    }

    Err(format!(
        "Failed to locate required binaries for platform '{}'. Expected: {} (yt-dlp), {} (aria2c), {} (ffmpeg) in directory: {}",
        plat, y_name, a_name, f_name, base_rel.display()
    ))
}

pub fn ensure_executable(paths: &BinaryPaths) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&paths.yt_dlp, &paths.aria2c, &paths.ffmpeg] {
            if let Ok(meta) = std::fs::metadata(p) {
                let mut perms = meta.permissions();
                let mode = perms.mode();
                if mode & 0o111 == 0 {
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
