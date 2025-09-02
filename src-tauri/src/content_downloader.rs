use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, Duration, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Window};

use crate::content_manifest::{ContentPack, Platform};
use crate::crypto::{CryptoManager, HashStatus, SignatureStatus};

/// Download progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentDownloadProgress {
    /// Pack ID being downloaded
    pub pack_id: String,
    
    /// Current download percentage (0-100)
    pub percentage: f64,
    
    /// Bytes downloaded so far
    pub bytes_downloaded: u64,
    
    /// Total bytes to download
    pub total_bytes: u64,
    
    /// Current download speed in bytes per second
    pub speed_bytes_per_sec: u64,
    
    /// Formatted speed string (e.g., "5.2 MB/s")
    pub speed_formatted: String,
    
    /// Estimated time remaining
    pub eta: String,
    
    /// Current download phase
    pub phase: DownloadPhase,
    
    /// Current status
    pub status: DownloadStatus,
    
    /// Error message if status is Error
    pub error_message: Option<String>,
    
    /// Download start time
    pub started_at: SystemTime,
    
    /// Whether download can be resumed
    pub resumable: bool,
}

/// Download phases
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadPhase {
    /// Preparing download
    Preparing,
    
    /// Downloading content
    Downloading,
    
    /// Verifying checksums
    Verifying,
    
    /// Verifying signatures
    SignatureCheck,
    
    /// Extracting archive
    Extracting,
    
    /// Installing files
    Installing,
    
    /// Cleaning up
    Cleanup,
    
    /// Complete
    Complete,
}

/// Download status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    /// Download is active
    Active,
    
    /// Download is paused
    Paused,
    
    /// Download completed successfully
    Completed,
    
    /// Download failed
    Error,
    
    /// Download was cancelled
    Cancelled,
}

/// Content downloader manager
pub struct ContentDownloader {
    /// Application handle
    app_handle: AppHandle,
    
    /// Content directory
    content_dir: PathBuf,
    
    /// Temporary downloads directory
    temp_dir: PathBuf,
    
    /// Crypto manager for verification
    crypto: CryptoManager,
    
    /// Active downloads
    active_downloads: Arc<Mutex<HashMap<String, Arc<Mutex<ContentDownloadProgress>>>>>,
    
    /// HTTP client
    client: reqwest::Client,
}

impl ContentDownloader {
    /// Create a new content downloader
    pub fn new(app_handle: AppHandle, content_dir: PathBuf) -> Result<Self, String> {
        let temp_dir = content_dir.join(".downloads");
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| format!("Failed to create downloads directory: {}", e))?;
        
        let crypto = CryptoManager::new();
        let active_downloads = Arc::new(Mutex::new(HashMap::new()));
        
        // Configure HTTP client with sensible defaults
        let client = reqwest::Client::builder()
            .user_agent("U-Download/2.2.0 (Content Downloader)")
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
        
        Ok(ContentDownloader {
            app_handle,
            content_dir,
            temp_dir,
            crypto,
            active_downloads,
            client,
        })
    }

    /// Start downloading a content pack
    pub async fn download_pack(
        &self,
        pack: &ContentPack,
        platform: &Platform,
        window: &Window,
    ) -> Result<(), String> {
        let pack_id = pack.id.clone();
        
        // Check if already downloading
        {
            let active = self.active_downloads.lock().unwrap();
            if active.contains_key(&pack_id) {
                return Err("Pack is already being downloaded".to_string());
            }
        }

        // Initialize progress tracking
        let progress = Arc::new(Mutex::new(ContentDownloadProgress {
            pack_id: pack_id.clone(),
            percentage: 0.0,
            bytes_downloaded: 0,
            total_bytes: platform.compressed_size,
            speed_bytes_per_sec: 0,
            speed_formatted: "0 B/s".to_string(),
            eta: "Calculating...".to_string(),
            phase: DownloadPhase::Preparing,
            status: DownloadStatus::Active,
            error_message: None,
            started_at: SystemTime::now(),
            resumable: true,
        }));

        // Register active download
        {
            let mut active = self.active_downloads.lock().unwrap();
            active.insert(pack_id.clone(), progress.clone());
        }

        // Clone necessary data for the async task
        let downloader = self.clone_for_async();
        let pack = pack.clone();
        let platform = platform.clone();
        let window = window.clone();

        // Spawn download task
        tokio::spawn(async move {
            let result = downloader.download_pack_impl(&pack, &platform, progress.clone()).await;
            
            // Update final status
            match result {
                Ok(_) => {
                    let mut prog = progress.lock().unwrap();
                    prog.status = DownloadStatus::Completed;
                    prog.phase = DownloadPhase::Complete;
                    prog.percentage = 100.0;
                    
                    // Emit completion event
                    let _ = window.emit("content-download-complete", prog.clone());
                }
                Err(e) => {
                    let mut prog = progress.lock().unwrap();
                    prog.status = DownloadStatus::Error;
                    prog.error_message = Some(e.clone());
                    
                    // Emit error event
                    let _ = window.emit("content-download-error", prog.clone());
                }
            }

            // Remove from active downloads
            {
                let mut active = downloader.active_downloads.lock().unwrap();
                active.remove(&pack.id);
            }
        });

        Ok(())
    }

    /// Clone downloader for async operations
    fn clone_for_async(&self) -> Self {
        ContentDownloader {
            app_handle: self.app_handle.clone(),
            content_dir: self.content_dir.clone(),
            temp_dir: self.temp_dir.clone(),
            crypto: CryptoManager::new(),
            active_downloads: self.active_downloads.clone(),
            client: self.client.clone(),
        }
    }

    /// Implementation of pack download
    async fn download_pack_impl(
        &self,
        pack: &ContentPack,
        platform: &Platform,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<(), String> {
        // Phase 1: Download archive
        let archive_path = self.download_archive(pack, platform, progress.clone()).await?;
        
        // Phase 2: Verify checksum
        self.verify_archive_checksum(&archive_path, platform, progress.clone()).await?;
        
        // Phase 3: Verify signature (if present)
        if let Some(signature) = &platform.signature {
            self.verify_archive_signature(&archive_path, signature, progress.clone()).await?;
        }
        
        // Phase 4: Extract archive
        let extracted_dir = self.extract_archive(&archive_path, platform, progress.clone()).await?;
        
        // Phase 5: Install files
        self.install_pack_files(pack, &extracted_dir, progress.clone()).await?;
        
        // Phase 6: Cleanup
        self.cleanup_download(&archive_path, &extracted_dir, progress.clone()).await?;
        
        Ok(())
    }

    /// Download the archive file with resumable support
    async fn download_archive(
        &self,
        pack: &ContentPack,
        platform: &Platform,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<PathBuf, String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::Downloading;
        }

        let archive_name = format!("u-download-content-{}-{}.{}", 
                                  platform.id, pack.version, platform.format);
        let archive_path = self.temp_dir.join(&archive_name);
        
        // Check if partial download exists
        let mut start_byte = 0;
        if archive_path.exists() {
            if let Ok(metadata) = std::fs::metadata(&archive_path) {
                start_byte = metadata.len();
                
                // Update progress for resume
                let mut prog = progress.lock().unwrap();
                prog.bytes_downloaded = start_byte;
                prog.percentage = (start_byte as f64 / platform.compressed_size as f64) * 100.0;
            }
        }

        // Build request with range header for resume
        let mut request = self.client.get(&platform.download_url);
        if start_byte > 0 {
            request = request.header("Range", format!("bytes={}-", start_byte));
        }

        let response = request.send().await
            .map_err(|e| format!("Failed to start download: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Download failed with status: {}", response.status()));
        }

        // Open file for writing (append mode if resuming)
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(start_byte > 0)
            .write(true)
            .truncate(start_byte == 0)
            .open(&archive_path)
            .map_err(|e| format!("Failed to create download file: {}", e))?;

        // Download with progress tracking
        let mut stream = response.bytes_stream();
        let mut last_update = SystemTime::now();
        let mut bytes_since_update = 0u64;

        while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
            
            use std::io::Write;
            file.write_all(&chunk)
                .map_err(|e| format!("Failed to write to file: {}", e))?;
            
            // Update progress
            bytes_since_update += chunk.len() as u64;
            let now = SystemTime::now();
            
            if now.duration_since(last_update).unwrap_or_default().as_millis() >= 250 {
                let mut prog = progress.lock().unwrap();
                prog.bytes_downloaded += bytes_since_update;
                prog.percentage = (prog.bytes_downloaded as f64 / platform.compressed_size as f64) * 100.0;
                
                // Calculate speed
                let duration = now.duration_since(last_update).unwrap_or_default();
                let secs = duration.as_secs_f64();
                if secs > 0.0 {
                    prog.speed_bytes_per_sec = (bytes_since_update as f64 / secs) as u64;
                    prog.speed_formatted = Self::format_speed(prog.speed_bytes_per_sec);
                    
                    // Calculate ETA
                    let remaining_bytes = platform.compressed_size.saturating_sub(prog.bytes_downloaded);
                    if prog.speed_bytes_per_sec > 0 {
                        let eta_seconds = remaining_bytes / prog.speed_bytes_per_sec;
                        prog.eta = Self::format_eta(eta_seconds);
                    }
                }
                
                // Emit progress event
                let _ = self.app_handle.emit("content-download-progress", prog.clone());
                
                last_update = now;
                bytes_since_update = 0;
            }
        }

        file.flush().map_err(|e| format!("Failed to flush file: {}", e))?;
        
        Ok(archive_path)
    }

    /// Verify archive checksum
    async fn verify_archive_checksum(
        &self,
        archive_path: &Path,
        platform: &Platform,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<(), String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::Verifying;
            let _ = self.app_handle.emit("content-download-progress", prog.clone());
        }

        match self.crypto.verify_file_hash(archive_path, &platform.sha256) {
            HashStatus::Valid => Ok(()),
            HashStatus::Invalid => Err("Archive checksum verification failed".to_string()),
            HashStatus::Error(e) => Err(format!("Checksum verification error: {}", e)),
        }
    }

    /// Verify archive signature
    async fn verify_archive_signature(
        &self,
        archive_path: &Path,
        signature: &str,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<(), String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::SignatureCheck;
            let _ = self.app_handle.emit("content-download-progress", prog.clone());
        }

        match self.crypto.verify_file_signature(archive_path, signature) {
            SignatureStatus::Valid => Ok(()),
            SignatureStatus::Invalid => Err("Archive signature verification failed".to_string()),
            SignatureStatus::Missing => Err("Archive signature is missing".to_string()),
            SignatureStatus::NoKey => Err("Public key not available for verification".to_string()),
            SignatureStatus::Error(e) => Err(format!("Signature verification error: {}", e)),
        }
    }

    /// Extract archive to temporary directory
    async fn extract_archive(
        &self,
        archive_path: &Path,
        platform: &Platform,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<PathBuf, String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::Extracting;
            let _ = self.app_handle.emit("content-download-progress", prog.clone());
        }

        let extract_dir = self.temp_dir.join(format!("extract-{}", 
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()));
        
        std::fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to create extraction directory: {}", e))?;

        match platform.format.as_str() {
            "tar.gz" => self.extract_tar_gz(archive_path, &extract_dir).await,
            "zip" => self.extract_zip(archive_path, &extract_dir).await,
            _ => Err(format!("Unsupported archive format: {}", platform.format)),
        }?;

        Ok(extract_dir)
    }

    /// Extract tar.gz archive
    async fn extract_tar_gz(&self, archive_path: &Path, extract_dir: &Path) -> Result<(), String> {
        use std::process::Command;
        
        let output = Command::new("tar")
            .args(&["-xzf", archive_path.to_str().unwrap()])
            .arg("-C")
            .arg(extract_dir.to_str().unwrap())
            .output()
            .map_err(|e| format!("Failed to run tar: {}", e))?;

        if !output.status.success() {
            return Err(format!("tar extraction failed: {}", 
                String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    }

    /// Extract zip archive
    async fn extract_zip(&self, archive_path: &Path, extract_dir: &Path) -> Result<(), String> {
        use std::process::Command;
        
        let output = Command::new("unzip")
            .arg("-q")
            .arg(archive_path.to_str().unwrap())
            .arg("-d")
            .arg(extract_dir.to_str().unwrap())
            .output()
            .map_err(|e| format!("Failed to run unzip: {}", e))?;

        if !output.status.success() {
            return Err(format!("unzip extraction failed: {}", 
                String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    }

    /// Install extracted files to final location
    async fn install_pack_files(
        &self,
        pack: &ContentPack,
        extracted_dir: &Path,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<(), String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::Installing;
            let _ = self.app_handle.emit("content-download-progress", prog.clone());
        }

        let pack_dir = self.content_dir.join(&pack.id);
        std::fs::create_dir_all(&pack_dir)
            .map_err(|e| format!("Failed to create pack directory: {}", e))?;

        // Install each file with verification
        for file in &pack.files {
            let src_path = extracted_dir.join(&file.path);
            let dest_path = pack_dir.join(&file.path);
            
            // Ensure destination directory exists
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }

            // Copy file
            self.crypto.secure_move(&src_path, &dest_path)
                .map_err(|e| format!("Failed to install file {}: {}", file.path, e))?;

            // Set executable permissions if needed
            #[cfg(unix)]
            if file.executable {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&dest_path)
                    .map_err(|e| format!("Failed to get file metadata: {}", e))?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&dest_path, perms)
                    .map_err(|e| format!("Failed to set executable permissions: {}", e))?;
            }

            // Verify file integrity
            match self.crypto.verify_file_hash(&dest_path, &file.sha256) {
                HashStatus::Valid => {},
                HashStatus::Invalid => {
                    return Err(format!("File {} failed integrity check", file.path));
                }
                HashStatus::Error(e) => {
                    return Err(format!("Failed to verify file {}: {}", file.path, e));
                }
            }
        }

        Ok(())
    }

    /// Cleanup temporary files
    async fn cleanup_download(
        &self,
        archive_path: &Path,
        extracted_dir: &Path,
        progress: Arc<Mutex<ContentDownloadProgress>>,
    ) -> Result<(), String> {
        // Update phase
        {
            let mut prog = progress.lock().unwrap();
            prog.phase = DownloadPhase::Cleanup;
            let _ = self.app_handle.emit("content-download-progress", prog.clone());
        }

        // Remove archive file
        if archive_path.exists() {
            std::fs::remove_file(archive_path)
                .map_err(|e| format!("Failed to remove archive: {}", e))?;
        }

        // Remove extraction directory
        if extracted_dir.exists() {
            std::fs::remove_dir_all(extracted_dir)
                .map_err(|e| format!("Failed to remove extraction directory: {}", e))?;
        }

        Ok(())
    }

    /// Format speed for display
    fn format_speed(bytes_per_sec: u64) -> String {
        const UNITS: &[&str] = &["B/s", "KB/s", "MB/s", "GB/s"];
        let mut speed = bytes_per_sec as f64;
        let mut unit_index = 0;

        while speed >= 1024.0 && unit_index < UNITS.len() - 1 {
            speed /= 1024.0;
            unit_index += 1;
        }

        format!("{:.1} {}", speed, UNITS[unit_index])
    }

    /// Format ETA for display
    fn format_eta(seconds: u64) -> String {
        if seconds < 60 {
            format!("{}s", seconds)
        } else if seconds < 3600 {
            format!("{}m {}s", seconds / 60, seconds % 60)
        } else {
            format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
        }
    }

    /// Get progress for a specific pack
    pub fn get_download_progress(&self, pack_id: &str) -> Option<ContentDownloadProgress> {
        let active = self.active_downloads.lock().unwrap();
        active.get(pack_id).map(|progress| progress.lock().unwrap().clone())
    }

    /// Cancel a download
    pub fn cancel_download(&self, pack_id: &str) -> Result<(), String> {
        let active = self.active_downloads.lock().unwrap();
        if let Some(progress) = active.get(pack_id) {
            let mut prog = progress.lock().unwrap();
            prog.status = DownloadStatus::Cancelled;
            Ok(())
        } else {
            Err("Download not found".to_string())
        }
    }

    /// Pause a download
    pub fn pause_download(&self, pack_id: &str) -> Result<(), String> {
        let active = self.active_downloads.lock().unwrap();
        if let Some(progress) = active.get(pack_id) {
            let mut prog = progress.lock().unwrap();
            prog.status = DownloadStatus::Paused;
            Ok(())
        } else {
            Err("Download not found".to_string())
        }
    }

    /// Resume a download
    pub fn resume_download(&self, pack_id: &str) -> Result<(), String> {
        let active = self.active_downloads.lock().unwrap();
        if let Some(progress) = active.get(pack_id) {
            let mut prog = progress.lock().unwrap();
            if prog.status == DownloadStatus::Paused {
                prog.status = DownloadStatus::Active;
                Ok(())
            } else {
                Err("Download is not paused".to_string())
            }
        } else {
            Err("Download not found".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speed_formatting() {
        assert_eq!(ContentDownloader::format_speed(1024), "1.0 KB/s");
        assert_eq!(ContentDownloader::format_speed(1048576), "1.0 MB/s");
        assert_eq!(ContentDownloader::format_speed(5242880), "5.0 MB/s");
    }

    #[test]
    fn test_eta_formatting() {
        assert_eq!(ContentDownloader::format_eta(30), "30s");
        assert_eq!(ContentDownloader::format_eta(90), "1m 30s");
        assert_eq!(ContentDownloader::format_eta(3661), "1h 1m");
    }
}