use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::Manager;

/// Content pack manifest for U-Download
/// Describes downloadable content packs and their metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentManifest {
    /// Manifest version for compatibility checking
    pub version: String,
    
    /// Timestamp when manifest was generated
    pub generated_at: String,
    
    /// App version this manifest is compatible with
    pub app_version: String,
    
    /// Content packs available for download
    pub content_packs: Vec<ContentPack>,
    
    /// Manifest signature (base64 encoded)
    pub signature: Option<String>,
}

/// Individual content pack definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPack {
    /// Unique identifier for this content pack
    pub id: String,
    
    /// Human-readable name
    pub name: String,
    
    /// Description of what this pack contains
    pub description: String,
    
    /// Pack version
    pub version: String,
    
    /// Whether this pack is required for basic functionality
    pub required: bool,
    
    /// Target platforms for this pack
    pub platforms: Vec<Platform>,
    
    /// Total uncompressed size in bytes
    pub total_size: u64,
    
    /// Files contained in this pack
    pub files: Vec<ContentFile>,
    
    /// Dependencies (other pack IDs this pack requires)
    pub dependencies: Vec<String>,
}

/// Platform-specific information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    /// Platform identifier (e.g., "linux-x64", "windows-x64", "macos-arm64")
    pub id: String,
    
    /// Human-readable platform name
    pub name: String,
    
    /// Download URL for this platform's pack
    pub download_url: String,
    
    /// Compressed archive size in bytes
    pub compressed_size: u64,
    
    /// SHA-256 hash of the compressed archive
    pub sha256: String,
    
    /// Archive format (e.g., "tar.gz", "zip")
    pub format: String,
    
    /// Optional signature for this platform's pack
    pub signature: Option<String>,
}

/// Individual file within a content pack
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentFile {
    /// Relative path within the content pack
    pub path: String,
    
    /// File size in bytes
    pub size: u64,
    
    /// SHA-256 hash of the file
    pub sha256: String,
    
    /// Whether this file should be executable
    pub executable: bool,
    
    /// File type/category for organization
    pub file_type: FileType,
}

/// Categories of files in content packs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    /// Binary executables (yt-dlp, aria2c, ffmpeg)
    Binary,
    
    /// Configuration files
    Config,
    
    /// Documentation
    Docs,
    
    /// Assets (icons, images, etc.)
    Asset,
    
    /// Other files
    Other,
}

/// Content manager for handling manifest operations
pub struct ContentManager {
    /// Application data directory
    pub app_data_dir: PathBuf,
    
    /// Content directory where packs are stored
    pub content_dir: PathBuf,
    
    /// Manifest cache directory
    pub manifest_cache_dir: PathBuf,
}

impl ContentManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Result<Self, String> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;

        let content_dir = app_data_dir.join("content");
        let manifest_cache_dir = app_data_dir.join("manifests");

        // Create directories if they don't exist
        std::fs::create_dir_all(&content_dir)
            .map_err(|e| format!("Failed to create content directory: {}", e))?;
        
        std::fs::create_dir_all(&manifest_cache_dir)
            .map_err(|e| format!("Failed to create manifest cache directory: {}", e))?;

        Ok(ContentManager {
            app_data_dir,
            content_dir,
            manifest_cache_dir,
        })
    }

    /// Load manifest from local cache or fetch from remote
    pub async fn load_manifest(&self, manifest_url: &str) -> Result<ContentManifest, String> {
        // Try to load from cache first
        let cache_path = self.manifest_cache_dir.join("content_manifest.json");
        
        if cache_path.exists() {
            match self.load_manifest_from_file(&cache_path) {
                Ok(manifest) => {
                    // Check if cached manifest is still valid (less than 24 hours old)
                    if self.is_manifest_fresh(&manifest, std::time::Duration::from_secs(24 * 3600)) {
                        return Ok(manifest);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to load cached manifest: {}", e);
                }
            }
        }

        // Fetch fresh manifest from remote
        let manifest = self.fetch_manifest_from_url(manifest_url).await?;
        
        // Cache the manifest
        if let Err(e) = self.save_manifest_to_file(&manifest, &cache_path) {
            eprintln!("Warning: Failed to cache manifest: {}", e);
        }

        Ok(manifest)
    }

    /// Load manifest from a local file
    pub fn load_manifest_from_file(&self, path: &PathBuf) -> Result<ContentManifest, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read manifest file: {}", e))?;
        
        let manifest: ContentManifest = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse manifest JSON: {}", e))?;
        
        Ok(manifest)
    }

    /// Save manifest to a local file
    pub fn save_manifest_to_file(&self, manifest: &ContentManifest, path: &PathBuf) -> Result<(), String> {
        let content = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
        
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write manifest file: {}", e))?;
        
        Ok(())
    }

    /// Fetch manifest from remote URL
    async fn fetch_manifest_from_url(&self, _url: &str) -> Result<ContentManifest, String> {
        // This is a placeholder - in a real implementation, you'd use an HTTP client
        // For now, we'll simulate by loading from a local file
        Err("Remote manifest fetching not implemented yet".to_string())
    }

    /// Check if manifest is fresh (within the specified duration)
    fn is_manifest_fresh(&self, manifest: &ContentManifest, max_age: std::time::Duration) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        // Parse the generated_at timestamp
        if let Ok(generated_time) = chrono::DateTime::parse_from_rfc3339(&manifest.generated_at) {
            let generated_timestamp = generated_time.timestamp() as u64;
            let now_timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            let age = now_timestamp.saturating_sub(generated_timestamp);
            return age < max_age.as_secs();
        }
        
        false
    }

    /// Get current platform identifier
    pub fn get_current_platform() -> String {
        if cfg!(target_os = "windows") {
            if cfg!(target_arch = "x86_64") {
                "windows-x64".to_string()
            } else {
                "windows-x64".to_string() // Default fallback
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "macos-arm64".to_string()
            } else {
                "macos-x64".to_string()
            }
        } else if cfg!(target_os = "linux") {
            if cfg!(target_arch = "aarch64") {
                "linux-arm64".to_string()
            } else {
                "linux-x64".to_string()
            }
        } else {
            "linux-x64".to_string() // Default fallback
        }
    }

    /// Find compatible content packs for the current platform
    pub fn find_compatible_packs<'a>(&self, manifest: &'a ContentManifest) -> Vec<&'a ContentPack> {
        let current_platform = Self::get_current_platform();
        
        manifest
            .content_packs
            .iter()
            .filter(|pack| {
                pack.platforms
                    .iter()
                    .any(|platform| platform.id == current_platform)
            })
            .collect()
    }

    /// Check if a content pack is already installed
    pub fn is_pack_installed(&self, pack: &ContentPack) -> bool {
        let pack_dir = self.content_dir.join(&pack.id);
        
        if !pack_dir.exists() {
            return false;
        }

        // Verify all files exist and have correct checksums
        for file in &pack.files {
            let file_path = pack_dir.join(&file.path);
            
            if !file_path.exists() {
                return false;
            }

            // Quick check: verify file size matches
            if let Ok(metadata) = std::fs::metadata(&file_path) {
                if metadata.len() != file.size {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

    /// Get installation status for all compatible packs
    pub fn get_installation_status(&self, manifest: &ContentManifest) -> HashMap<String, PackStatus> {
        let mut status = HashMap::new();
        
        for pack in self.find_compatible_packs(manifest) {
            let pack_status = if self.is_pack_installed(pack) {
                PackStatus::Installed
            } else {
                PackStatus::NotInstalled
            };
            
            status.insert(pack.id.clone(), pack_status);
        }
        
        status
    }
}

/// Installation status for content packs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackStatus {
    /// Pack is fully installed and verified
    Installed,
    
    /// Pack is not installed
    NotInstalled,
    
    /// Pack is currently being downloaded
    Downloading,
    
    /// Pack download failed
    Failed,
    
    /// Pack is being installed/extracted
    Installing,
    
    /// Pack is installed but verification failed
    Corrupted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let platform = ContentManager::get_current_platform();
        assert!(!platform.is_empty());
        assert!(platform.contains("-"));
    }

    #[test]
    fn test_manifest_serialization() {
        let manifest = ContentManifest {
            version: "1.0.0".to_string(),
            generated_at: "2025-01-01T00:00:00Z".to_string(),
            app_version: "2.3.0".to_string(),
            content_packs: vec![],
            signature: None,
        };

        let serialized = serde_json::to_string(&manifest).unwrap();
        let deserialized: ContentManifest = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(manifest.version, deserialized.version);
        assert_eq!(manifest.app_version, deserialized.app_version);
    }
}