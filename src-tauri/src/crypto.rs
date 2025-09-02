use serde::{Deserialize, Serialize};
use std::path::Path;
use base64::{Engine as _, engine::general_purpose};

/// Cryptographic operations for content signing and verification
pub struct CryptoManager {
    /// Public key for verification (embedded in app)
    public_key: Option<Vec<u8>>,
    
    /// Private key for signing (only used during build/release)
    private_key: Option<Vec<u8>>,
}

/// Signature verification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignatureStatus {
    /// Signature is valid
    Valid,
    
    /// Signature is invalid
    Invalid,
    
    /// No signature present
    Missing,
    
    /// Public key not available
    NoKey,
    
    /// Error during verification
    Error(String),
}

/// Hash verification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HashStatus {
    /// Hash matches expected value
    Valid,
    
    /// Hash does not match
    Invalid,
    
    /// Error computing hash
    Error(String),
}

impl CryptoManager {
    /// Create a new crypto manager with embedded public key
    pub fn new() -> Self {
        // In production, this would be the actual public key
        // For now, using a placeholder
        let public_key = include_bytes!("../assets/public_key.pem").to_vec();
        
        CryptoManager {
            public_key: Some(public_key),
            private_key: None,
        }
    }

    /// Create a crypto manager for signing (development/CI only)
    pub fn with_private_key(private_key_path: &Path) -> Result<Self, String> {
        let private_key = std::fs::read(private_key_path)
            .map_err(|e| format!("Failed to read private key: {}", e))?;
        
        let public_key = include_bytes!("../assets/public_key.pem").to_vec();
        
        Ok(CryptoManager {
            public_key: Some(public_key),
            private_key: Some(private_key),
        })
    }

    /// Compute SHA-256 hash of a file
    pub fn compute_file_hash(&self, file_path: &Path) -> Result<String, String> {
        use sha2::{Sha256, Digest};
        use std::io::Read;
        
        let mut file = std::fs::File::open(file_path)
            .map_err(|e| format!("Failed to open file: {}", e))?;
        
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];
        
        loop {
            let bytes_read = file.read(&mut buffer)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            
            if bytes_read == 0 {
                break;
            }
            
            hasher.update(&buffer[..bytes_read]);
        }
        
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Compute SHA-256 hash of data
    pub fn compute_data_hash(&self, data: &[u8]) -> String {
        use sha2::{Sha256, Digest};
        
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Verify SHA-256 hash of a file
    pub fn verify_file_hash(&self, file_path: &Path, expected_hash: &str) -> HashStatus {
        match self.compute_file_hash(file_path) {
            Ok(actual_hash) => {
                if actual_hash.to_lowercase() == expected_hash.to_lowercase() {
                    HashStatus::Valid
                } else {
                    HashStatus::Invalid
                }
            }
            Err(e) => HashStatus::Error(e),
        }
    }

    /// Sign data with private key (for CI/build systems)
    pub fn sign_data(&self, data: &[u8]) -> Result<String, String> {
        let private_key = self.private_key.as_ref()
            .ok_or("Private key not available")?;

        // This is a simplified signing implementation
        // In production, you'd use proper cryptographic libraries like ring, ed25519-dalek, etc.
        
        // For now, create a simple HMAC-based signature
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        
        type HmacSha256 = Hmac<Sha256>;
        
        let mut mac = HmacSha256::new_from_slice(private_key)
            .map_err(|e| format!("Invalid key: {}", e))?;
        
        mac.update(data);
        let result = mac.finalize();
        
        // Return base64 encoded signature
        Ok(general_purpose::STANDARD.encode(result.into_bytes()))
    }

    /// Verify signature with public key
    pub fn verify_signature(&self, data: &[u8], signature: &str) -> SignatureStatus {
        let public_key = match &self.public_key {
            Some(key) => key,
            None => return SignatureStatus::NoKey,
        };

        // Decode base64 signature
        let signature_bytes = match general_purpose::STANDARD.decode(signature) {
            Ok(bytes) => bytes,
            Err(e) => return SignatureStatus::Error(format!("Invalid base64 signature: {}", e)),
        };

        // Verify using HMAC (simplified approach)
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        
        type HmacSha256 = Hmac<Sha256>;
        
        let mut mac = match HmacSha256::new_from_slice(public_key) {
            Ok(mac) => mac,
            Err(e) => return SignatureStatus::Error(format!("Invalid public key: {}", e)),
        };
        
        mac.update(data);
        
        match mac.verify_slice(&signature_bytes) {
            Ok(_) => SignatureStatus::Valid,
            Err(_) => SignatureStatus::Invalid,
        }
    }

    /// Sign a JSON string (for manifests)
    pub fn sign_json(&self, json_str: &str) -> Result<String, String> {
        self.sign_data(json_str.as_bytes())
    }

    /// Verify a JSON string signature
    pub fn verify_json_signature(&self, json_str: &str, signature: &str) -> SignatureStatus {
        self.verify_signature(json_str.as_bytes(), signature)
    }

    /// Sign a file
    pub fn sign_file(&self, file_path: &Path) -> Result<String, String> {
        let data = std::fs::read(file_path)
            .map_err(|e| format!("Failed to read file: {}", e))?;
        
        self.sign_data(&data)
    }

    /// Verify file signature
    pub fn verify_file_signature(&self, file_path: &Path, signature: &str) -> SignatureStatus {
        let data = match std::fs::read(file_path) {
            Ok(data) => data,
            Err(e) => return SignatureStatus::Error(format!("Failed to read file: {}", e)),
        };
        
        self.verify_signature(&data, signature)
    }

    /// Create a secure temporary directory for downloads
    pub fn create_secure_temp_dir(&self) -> Result<std::path::PathBuf, String> {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let temp_dir = std::env::temp_dir()
            .join("u-download-secure")
            .join(format!("download-{}", timestamp));
        
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| format!("Failed to create secure temp directory: {}", e))?;
        
        Ok(temp_dir)
    }

    /// Secure file move operation (atomic when possible)
    pub fn secure_move(&self, from: &Path, to: &Path) -> Result<(), String> {
        // Ensure destination directory exists
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create destination directory: {}", e))?;
        }

        // Try atomic move first
        if std::fs::rename(from, to).is_ok() {
            return Ok(());
        }

        // Fallback to copy + remove
        std::fs::copy(from, to)
            .map_err(|e| format!("Failed to copy file: {}", e))?;
        
        std::fs::remove_file(from)
            .map_err(|e| format!("Failed to remove source file: {}", e))?;
        
        Ok(())
    }

    /// Validate path is safe (no directory traversal)
    pub fn validate_safe_path(&self, path: &str) -> Result<(), String> {
        let path = Path::new(path);
        
        // Check for absolute paths
        if path.is_absolute() {
            return Err("Absolute paths not allowed".to_string());
        }
        
        // Check for directory traversal attempts
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    return Err("Parent directory references not allowed".to_string());
                }
                std::path::Component::CurDir => {
                    // Current directory is OK
                }
                std::path::Component::Normal(_) => {
                    // Normal path components are OK
                }
                _ => {
                    return Err("Invalid path component".to_string());
                }
            }
        }
        
        Ok(())
    }
}

impl Default for CryptoManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate key pair for signing (development utility)
pub fn generate_key_pair(output_dir: &Path) -> Result<(), String> {
    // This would generate a real Ed25519 key pair in production
    // For now, generate simple HMAC keys
    
    use rand::RngCore;
    
    let mut private_key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut private_key);
    
    let public_key = private_key.clone(); // In HMAC, public and private are the same
    
    let private_key_path = output_dir.join("private_key.pem");
    let public_key_path = output_dir.join("public_key.pem");
    
    std::fs::write(&private_key_path, &private_key)
        .map_err(|e| format!("Failed to write private key: {}", e))?;
    
    std::fs::write(&public_key_path, &public_key)
        .map_err(|e| format!("Failed to write public key: {}", e))?;
    
    println!("Generated key pair:");
    println!("  Private key: {}", private_key_path.display());
    println!("  Public key: {}", public_key_path.display());
    println!("");
    println!("⚠️  Keep the private key secure! It should only be used in CI/build systems.");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_hash_computation() {
        let crypto = CryptoManager::new();
        let data = b"hello world";
        let hash = crypto.compute_data_hash(data);
        
        // SHA-256 of "hello world"
        assert_eq!(hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    }

    #[test]
    fn test_path_validation() {
        let crypto = CryptoManager::new();
        
        // Valid paths
        assert!(crypto.validate_safe_path("file.txt").is_ok());
        assert!(crypto.validate_safe_path("dir/file.txt").is_ok());
        assert!(crypto.validate_safe_path("./file.txt").is_ok());
        
        // Invalid paths
        assert!(crypto.validate_safe_path("../file.txt").is_err());
        assert!(crypto.validate_safe_path("/absolute/path").is_err());
        assert!(crypto.validate_safe_path("dir/../../../file.txt").is_err());
    }

    #[test]
    fn test_temp_dir_creation() {
        let crypto = CryptoManager::new();
        let temp_dir = crypto.create_secure_temp_dir().unwrap();
        
        assert!(temp_dir.exists());
        assert!(temp_dir.is_dir());
        
        // Clean up
        let _ = fs::remove_dir_all(temp_dir);
    }
}