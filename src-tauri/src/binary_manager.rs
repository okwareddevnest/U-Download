use std::path::{Path, PathBuf};
use std::fs;
use tauri::Manager;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone)]
pub struct BinaryManager {
    pub app_data_dir: PathBuf,
    pub temp_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BinaryPaths {
    pub yt_dlp: PathBuf,
    pub aria2c: PathBuf,
    pub ffmpeg: PathBuf,
}

impl BinaryManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Result<Self, String> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;

        // Create app data directory if it doesn't exist
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir)
                .map_err(|e| format!("Failed to create app data directory: {}", e))?;
        }

        // Use content directory for downloaded binaries
        let temp_dir = app_data_dir.join("content").join("core-binaries");
        if !temp_dir.exists() {
            fs::create_dir_all(&temp_dir)
                .map_err(|e| format!("Failed to create content binaries directory: {}", e))?;
        }

        Ok(BinaryManager {
            app_data_dir,
            temp_dir,
        })
    }

    pub fn get_binary_paths(&self) -> Result<BinaryPaths, String> {
        let yt_dlp = self.temp_dir.join(format!("yt-dlp{}", self.get_exe_suffix()));
        let aria2c = self.temp_dir.join(format!("aria2c{}", self.get_exe_suffix()));
        let ffmpeg = self.temp_dir.join(format!("ffmpeg{}", self.get_exe_suffix()));

        // In the new system, check if binaries are available from content packs
        // If not, provide helpful error message
        if !self.are_content_binaries_available() {
            return Err("Essential binaries not found. Please download the Core Content Pack from the first-run setup or settings panel.".to_string());
        }

        Ok(BinaryPaths {
            yt_dlp,
            aria2c,
            ffmpeg,
        })
    }

    /// Check if binaries are available from content packs
    fn are_content_binaries_available(&self) -> bool {
        let yt_dlp = self.temp_dir.join(format!("yt-dlp{}", self.get_exe_suffix()));
        let aria2c = self.temp_dir.join(format!("aria2c{}", self.get_exe_suffix()));
        let ffmpeg = self.temp_dir.join(format!("ffmpeg{}", self.get_exe_suffix()));

        yt_dlp.exists() && aria2c.exists() && ffmpeg.exists()
    }

    fn get_current_platform(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            if cfg!(target_arch = "x86_64") {
                "windows-x64"
            } else {
                "windows-x64" // Default fallback
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "macos-arm64"
            } else {
                "macos-x64"
            }
        } else if cfg!(target_os = "linux") {
            if cfg!(target_arch = "aarch64") {
                "linux-arm64"
            } else {
                "linux-x64"
            }
        } else {
            "linux-x64" // Default fallback
        }
    }

    fn get_exe_suffix(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        }
    }

    fn ensure_binaries_extracted(&self) -> Result<(), String> {
        // In the new content system, binaries are installed by the content downloader
        // This method now just ensures they have correct permissions if they exist
        
        let yt_dlp_path = self.temp_dir.join(format!("yt-dlp{}", self.get_exe_suffix()));
        let aria2c_path = self.temp_dir.join(format!("aria2c{}", self.get_exe_suffix()));
        let ffmpeg_path = self.temp_dir.join(format!("ffmpeg{}", self.get_exe_suffix()));

        // Ensure binaries are executable on Unix systems if they exist
        #[cfg(unix)]
        {
            if yt_dlp_path.exists() {
                self.make_executable(&yt_dlp_path)?;
            }
            if aria2c_path.exists() {
                self.make_executable(&aria2c_path)?;
            }
            if ffmpeg_path.exists() {
                self.make_executable(&ffmpeg_path)?;
            }
        }

        Ok(())
    }

    fn extract_platform_binaries(&self) -> Result<(), String> {
        // In the new content system, binaries are no longer bundled with the app
        // Instead, they should be downloaded via the content download system
        // This is a fallback that will suggest using the content system
        
        Err("Essential binaries are not bundled with the application. Please use the content download system to install required components.".to_string())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn extract_linux_x64_binaries(&self) -> Result<(), String> {
        // Extract embedded binaries for Linux x64
        self.extract_binary("yt-dlp", include_bytes!("../binaries/linux-x64/yt-dlp"))?;
        self.extract_binary("aria2c", include_bytes!("../binaries/linux-x64/aria2c"))?;
        self.extract_binary("ffmpeg", include_bytes!("../binaries/linux-x64/ffmpeg"))?;
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    fn extract_linux_arm64_binaries(&self) -> Result<(), String> {
        // Extract embedded binaries for Linux ARM64
        self.extract_binary("yt-dlp", include_bytes!("../binaries/linux-arm64/yt-dlp"))?;
        self.extract_binary("aria2c", include_bytes!("../binaries/linux-arm64/aria2c"))?;
        self.extract_binary("ffmpeg", include_bytes!("../binaries/linux-arm64/ffmpeg"))?;
        Ok(())
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn extract_windows_x64_binaries(&self) -> Result<(), String> {
        // Extract embedded binaries for Windows x64
        self.extract_binary("yt-dlp.exe", include_bytes!("../binaries/windows-x64/yt-dlp.exe"))?;
        self.extract_binary("aria2c.exe", include_bytes!("../binaries/windows-x64/aria2c.exe"))?;
        self.extract_binary("ffmpeg.exe", include_bytes!("../binaries/windows-x64/ffmpeg.exe"))?;
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    fn extract_macos_x64_binaries(&self) -> Result<(), String> {
        // Extract embedded binaries for macOS x64
        self.extract_binary("yt-dlp", include_bytes!("../binaries/macos-x64/yt-dlp"))?;
        self.extract_binary("aria2c", include_bytes!("../binaries/macos-x64/aria2c"))?;
        self.extract_binary("ffmpeg", include_bytes!("../binaries/macos-x64/ffmpeg"))?;
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn extract_macos_arm64_binaries(&self) -> Result<(), String> {
        // Extract embedded binaries for macOS ARM64
        self.extract_binary("yt-dlp", include_bytes!("../binaries/macos-arm64/yt-dlp"))?;
        self.extract_binary("aria2c", include_bytes!("../binaries/macos-arm64/aria2c"))?;
        self.extract_binary("ffmpeg", include_bytes!("../binaries/macos-arm64/ffmpeg"))?;
        Ok(())
    }

    fn extract_binary(&self, name: &str, data: &[u8]) -> Result<(), String> {
        let path = self.temp_dir.join(name);
        
        let mut file = fs::File::create(&path)
            .map_err(|e| format!("Failed to create binary file {}: {}", name, e))?;
        
        use std::io::Write;
        file.write_all(data)
            .map_err(|e| format!("Failed to write binary data for {}: {}", name, e))?;
        
        eprintln!("Extracted {} ({} bytes) to {}", name, data.len(), path.display());
        Ok(())
    }

    #[cfg(unix)]
    fn make_executable(&self, path: &Path) -> Result<(), String> {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Failed to get metadata for {}: {}", path.display(), e))?;
        
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755); // rwxr-xr-x
        
        fs::set_permissions(path, permissions)
            .map_err(|e| format!("Failed to set permissions for {}: {}", path.display(), e))?;
        
        Ok(())
    }

    pub fn verify_binaries(&self) -> Result<String, String> {
        let mut results = Vec::new();

        // Check if binaries are available
        if !self.are_content_binaries_available() {
            results.push("⚠️  Core binaries not found. Download the Core Content Pack to enable full functionality.".to_string());
            results.push("   • yt-dlp - Required for YouTube downloads".to_string());
            results.push("   • aria2c - Required for high-speed downloads".to_string());
            results.push("   • FFmpeg - Required for video trimming".to_string());
            return Ok(results.join("\n"));
        }

        let paths = self.get_binary_paths()?;

        // Test yt-dlp
        match std::process::Command::new(&paths.yt_dlp).arg("--version").output() {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                results.push(format!("✅ yt-dlp (content pack): {}", version.trim()));
            }
            Err(e) => {
                results.push(format!("❌ yt-dlp (content pack): {}", e));
            }
        }

        // Test aria2c
        match std::process::Command::new(&paths.aria2c).arg("--version").output() {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                results.push(format!(
                    "✅ aria2c (content pack): {}",
                    version.lines().next().unwrap_or("unknown")
                ));
            }
            Err(e) => {
                results.push(format!("❌ aria2c (content pack): {}", e));
            }
        }

        // Test ffmpeg
        match std::process::Command::new(&paths.ffmpeg).arg("-version").output() {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                results.push(format!(
                    "✅ FFmpeg (content pack): {}",
                    version.lines().next().unwrap_or("unknown")
                ));
            }
            Err(e) => {
                results.push(format!("❌ FFmpeg (content pack): {}", e));
            }
        }

        Ok(results.join("\n"))
    }
}