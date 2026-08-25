use std::path::{Path, PathBuf};
use tauri::{path::BaseDirectory, AppHandle, Manager, Runtime};

#[derive(Debug, Clone)]
pub struct BinaryPaths {
    pub dir: PathBuf,
    pub yt_dlp: PathBuf,
    pub aria2c: PathBuf,
    pub ffmpeg: PathBuf,
    /// The bundled JavaScript runtime, when this install has one.
    ///
    /// Optional on purpose. Recent yt-dlp needs a JS runtime to extract
    /// YouTube formats at all, but installs that predate the deno addition —
    /// and platforms whose deno has not been fetched yet — must keep working
    /// exactly as before rather than refusing to start. Its absence therefore
    /// never fails resolution; see `resolve_js_runtime` for the fallback.
    pub deno: Option<PathBuf>,
    /// The CA certificate bundle the child processes should trust, if one
    /// could be found at all.
    ///
    /// The bundled ffmpeg is built against OpenSSL and ships no trust store,
    /// so without this every HTTPS URL it opens fails verification — which is
    /// the whole trim path, since yt-dlp hands `--download-sections` fetches
    /// to ffmpeg. Optional for the same reason `deno` is: an install made
    /// before `cacert.pem` was added to the fetch script, on a machine with no
    /// system trust store either, must still start. See `resolve_ca_bundle`.
    pub ca_cert: Option<PathBuf>,
}

/// A JavaScript runtime yt-dlp can be pointed at, as the name yt-dlp knows it
/// by plus the executable's location.
///
/// yt-dlp enables only `deno` by default, so even a runtime already installed
/// on the machine has to be named explicitly on the command line.
#[derive(Debug, Clone)]
pub struct JsRuntime {
    pub name: &'static str,
    pub path: PathBuf,
}

impl JsRuntime {
    /// The value yt-dlp's `--js-runtimes` flag takes: `<name>:<path>`.
    pub fn flag_value(&self) -> String {
        format!("{}:{}", self.name, self.path.display())
    }
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

/// Looks for the bundled JS runtime in the directory yt-dlp was found in.
///
/// Returns `None` when the file is not there, which is the expected state for
/// every install made before deno was added to the fetch script. Resolution of
/// the three required binaries is unaffected either way.
fn optional_deno_beside(yt_dlp: &Path) -> Option<PathBuf> {
    let candidate = yt_dlp.parent()?.join(exe_name("deno"));
    if candidate.exists() {
        eprintln!("✅ Found bundled JS runtime: {}", candidate.display());
        Some(candidate)
    } else {
        None
    }
}

/// The filename `scripts/fetch-binaries.sh` installs the CA bundle under, in
/// the same directory as the binaries.
pub const CA_BUNDLE_NAME: &str = "cacert.pem";

/// Trust stores the host OS may provide, tried in order only when this install
/// has no bundled one. macOS/BSD first, then Debian/Ubuntu, then RHEL/Fedora.
///
/// Deliberately the fallback rather than the primary mechanism: these paths
/// differ per distribution, are absent from some minimal images, and none of
/// them exist on Windows at all. Only the bundled file behaves identically
/// everywhere.
const SYSTEM_CA_CANDIDATES: [&str; 3] = [
    "/etc/ssl/cert.pem",
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
];

/// The resolution order itself, with the system candidate list injected so it
/// can be exercised in a test without depending on what the host happens to
/// have in /etc.
fn resolve_ca_bundle_from(dir: &Path, system_candidates: &[&str]) -> Option<PathBuf> {
    let bundled = dir.join(CA_BUNDLE_NAME);
    if bundled.is_file() {
        return Some(bundled);
    }
    system_candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

/// Picks the CA bundle to point OpenSSL-backed children at: the one bundled
/// beside the binaries, else the first system trust store that exists, else
/// `None`.
///
/// `None` is a supported outcome, not an error: the child then runs with
/// exactly the environment it had before this existed. Degrading to today's
/// (broken-on-HTTPS) behaviour is still better than refusing to launch an
/// install that predates the bundled file.
pub fn resolve_ca_bundle(dir: &Path) -> Option<PathBuf> {
    resolve_ca_bundle_from(dir, &SYSTEM_CA_CANDIDATES)
}

/// Assembles a `BinaryPaths` once the three required binaries have been
/// located, resolving the optional extras that live beside them (the JS
/// runtime and the CA bundle) in one place so every resolution strategy
/// produces an identically-populated value.
fn assemble_paths(dir: PathBuf, yt_dlp: PathBuf, aria2c: PathBuf, ffmpeg: PathBuf) -> BinaryPaths {
    let deno = optional_deno_beside(&yt_dlp);
    let ca_cert = resolve_ca_bundle(&dir);
    match ca_cert.as_ref() {
        Some(ca) => eprintln!("\u{1f512} Using CA bundle: {}", ca.display()),
        None => eprintln!(
            "\u{26a0}\u{fe0f}  No CA bundle found (no {} beside the binaries, none of {} present). \
             HTTPS fetches made by ffmpeg may fail certificate verification.",
            CA_BUNDLE_NAME,
            SYSTEM_CA_CANDIDATES.join(", ")
        ),
    }
    BinaryPaths { dir, yt_dlp, aria2c, ffmpeg, deno, ca_cert }
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
            return Some(assemble_paths(dir, yt, ar, ff));
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
            return Some(assemble_paths(dir, yt, ar, ff));
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
            return Some(assemble_paths(dir, yt, ar, ff));
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
                return Some(assemble_paths(target_binaries_dir, yt, ar, ff));
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
                return Some(assemble_paths(parent.to_path_buf(), direct_path, ar, ff));
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
                    return Some(assemble_paths(parent.to_path_buf(), abs_path, ar, ff));
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

/// Names of the JS runtimes yt-dlp supports, in yt-dlp's own priority order.
/// `quickjs` is omitted: it is not something a machine is likely to have on
/// PATH under that name, and the fallback exists for interpreters that are.
const PATH_RUNTIME_CANDIDATES: [&str; 3] = ["deno", "node", "bun"];

/// Looks one executable up on `PATH`, the way a shell would.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let file = exe_name(name);
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(&file);
        if !candidate.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = std::fs::metadata(&candidate)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if !executable {
                continue;
            }
        }
        return Some(candidate);
    }
    None
}

/// Picks the JavaScript runtime to hand yt-dlp, or `None` if the machine has
/// none.
///
/// Recent yt-dlp cannot extract YouTube formats without one — with no runtime
/// it reports zero muxed formats, which makes preview fall back to a proxy
/// download that then fails too, and plain downloads fail as "Requested format
/// is not available".
///
/// Order: the bundled deno first (known-good version, no user setup), then any
/// runtime already installed on the machine. Returning `None` is a supported
/// outcome — callers then invoke yt-dlp exactly as they did before this
/// existed, so a missing runtime degrades quality rather than breaking the app.
pub fn resolve_js_runtime(paths: &BinaryPaths) -> Option<JsRuntime> {
    if let Some(path) = paths.deno.clone() {
        return Some(JsRuntime { name: "deno", path });
    }

    for name in PATH_RUNTIME_CANDIDATES {
        if let Some(path) = find_on_path(name) {
            eprintln!("🔧 Using JS runtime from PATH: {} ({})", name, path.display());
            return Some(JsRuntime { name, path });
        }
    }

    eprintln!(
        "⚠️  No JavaScript runtime found (bundled deno absent; none of {} on PATH). \
         YouTube extraction may return no usable formats.",
        PATH_RUNTIME_CANDIDATES.join(", ")
    );
    None
}

/// Appends `--js-runtimes <name>:<path>` when a runtime is available.
///
/// A single place so that every yt-dlp invocation spells the flag identically;
/// an app where preview passes it and download does not is worse than one that
/// passes it nowhere.
pub fn push_js_runtime_args(args: &mut Vec<String>, runtime: Option<&JsRuntime>) {
    if let Some(runtime) = runtime {
        args.push("--js-runtimes".to_string());
        args.push(runtime.flag_value());
    }
}

/// Ensure binaries have executable permissions on Unix systems
pub fn ensure_executable(paths: &BinaryPaths) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut targets: Vec<(&str, &PathBuf)> = vec![
            ("yt-dlp", &paths.yt_dlp),
            ("aria2c", &paths.aria2c),
            ("ffmpeg", &paths.ffmpeg),
        ];
        // Only when this install actually bundles one; a missing runtime is a
        // supported state, not something to chmod or complain about.
        if let Some(deno) = paths.deno.as_ref() {
            targets.push(("deno", deno));
        }
        for (name, p) in targets {
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

/// Prepares the environment of a command that will run one of the bundled
/// tools: the binary directory on `PATH`, and a CA bundle OpenSSL can find.
///
/// Both halves matter to yt-dlp specifically, because yt-dlp spawns ffmpeg as
/// a child and the child inherits this environment.
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

    apply_ca_env(cmd, dir);
}

/// Sets `SSL_CERT_FILE` so the OpenSSL-backed ffmpeg this command will run (or
/// spawn) has a trust store to verify against.
///
/// Without it the bundled ffmpeg fails every HTTPS input with
/// "error:0A000086:SSL routines::certificate verify failed", because the build
/// is `--enable-openssl` and carries no certificates. That takes the trim path
/// down with it: yt-dlp performs `--download-sections` fetches through ffmpeg.
///
/// `SSL_CERT_DIR` is deliberately left alone. It names a directory of
/// hash-named certificate links, which is not what either the bundled file's
/// directory or a bundle file's parent is; setting it would replace OpenSSL's
/// own default directory with a useless one for no gain, since `SSL_CERT_FILE`
/// is what resolves the lookup here.
fn apply_ca_env(cmd: &mut std::process::Command, dir: &Path) {
    // A trust store the user configured themselves wins: it is already in this
    // process's environment, the child inherits it untouched, and overriding it
    // would break anyone pointing at a corporate/proxy CA on purpose.
    if let Some(existing) = std::env::var_os("SSL_CERT_FILE") {
        if !existing.is_empty() {
            eprintln!(
                "🔒 Honouring SSL_CERT_FILE already set in the environment: {}",
                Path::new(&existing).display()
            );
            return;
        }
    }

    match resolve_ca_bundle(dir) {
        Some(ca) => {
            eprintln!("🔒 SSL_CERT_FILE={}", ca.display());
            cmd.env("SSL_CERT_FILE", ca);
        }
        // Degrade, never fail: an install predating the bundled cacert.pem, on
        // a machine with no system trust store either, runs exactly as it did
        // before this existed.
        None => eprintln!(
            "⚠️  No CA bundle to point SSL_CERT_FILE at (looked for {} in {}, then {}). \
             HTTPS fetches made by ffmpeg may fail certificate verification.",
            CA_BUNDLE_NAME,
            dir.display(),
            SYSTEM_CA_CANDIDATES.join(", ")
        ),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory of this test's own. `tempfile` is not a dependency
    /// of this crate and this is not worth adding one for: a per-process,
    /// per-call name under the system temp dir is enough for path-shape tests.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "udl-ca-test-{}-{}-{}",
                tag,
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("scratch dir should be creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let file = self.0.join(name);
            std::fs::write(&file, contents).expect("scratch file should be writable");
            file
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn ca_resolution_prefers_the_bundled_file() {
        let bundle_dir = ScratchDir::new("bundled");
        let bundled = bundle_dir.write(CA_BUNDLE_NAME, "-----BEGIN CERTIFICATE-----\n");

        let system = ScratchDir::new("system");
        let system_store = system.write("ca-certificates.crt", "-----BEGIN CERTIFICATE-----\n");
        let candidates = [system_store.to_str().unwrap()];

        assert_eq!(
            resolve_ca_bundle_from(bundle_dir.path(), &candidates),
            Some(bundled),
            "the bundled cacert.pem must win over an existing system trust store"
        );
    }

    #[test]
    fn ca_resolution_falls_back_to_the_first_existing_system_store() {
        // No cacert.pem beside the binaries: the state of every install made
        // before the fetch script started shipping one.
        let bundle_dir = ScratchDir::new("no-bundled");

        let system = ScratchDir::new("system-order");
        let second = system.write("ca-bundle.crt", "-----BEGIN CERTIFICATE-----\n");
        let missing = system.path().join("does-not-exist.pem");
        let third = system.write("cert.pem", "-----BEGIN CERTIFICATE-----\n");

        let candidates = [
            missing.to_str().unwrap(),
            second.to_str().unwrap(),
            third.to_str().unwrap(),
        ];

        assert_eq!(
            resolve_ca_bundle_from(bundle_dir.path(), &candidates),
            Some(second),
            "the first candidate that exists wins, and missing ones are skipped"
        );
    }

    #[test]
    fn ca_resolution_yields_none_when_nothing_exists() {
        let bundle_dir = ScratchDir::new("nothing");
        let absent = bundle_dir.path().join("nowhere.pem");

        assert_eq!(
            resolve_ca_bundle_from(bundle_dir.path(), &[absent.to_str().unwrap()]),
            None,
            "no bundled file and no system store must degrade to None, not panic or invent a path"
        );
    }

    #[test]
    fn a_directory_named_like_the_bundle_is_not_mistaken_for_it() {
        let bundle_dir = ScratchDir::new("dir-not-file");
        std::fs::create_dir_all(bundle_dir.path().join(CA_BUNDLE_NAME))
            .expect("directory should be creatable");

        assert_eq!(
            resolve_ca_bundle_from(bundle_dir.path(), &[]),
            None,
            "only a regular file counts as a bundle"
        );
    }
}
