//! Hash Calculation Module
//!
//! This module provides hash calculation functionality for LTFS file write operations.
//! Supports multiple hash algorithms: SHA1, MD5, SHA256, BLAKE3, XxHash3, XxHash128.

use super::WriteOptions;
use std::collections::HashMap;

/// LTFSCopyGUI compatible hash calculator
/// Corresponds to VB.NET CheckSumBlockwiseCalculator
pub struct CheckSumBlockwiseCalculator {
    sha1_hasher: sha1::Sha1,
    md5_hasher: md5::Context,
    sha256_hasher: sha2::Sha256,
    blake3_hasher: Option<blake3::Hasher>,
    xxh3_hasher: Option<xxhash_rust::xxh3::Xxh3>,
    xxh128_hasher: Option<xxhash_rust::xxh3::Xxh3>,
    bytes_processed: u64,
}

impl CheckSumBlockwiseCalculator {
    /// Create new hash calculator based on WriteOptions configuration
    pub fn new_with_options(options: &WriteOptions) -> Self {
        use sha1::Digest as Sha1Digest;
        use sha2::Digest as Sha256Digest;

        Self {
            sha1_hasher: Sha1Digest::new(),
            md5_hasher: md5::Context::new(),
            sha256_hasher: Sha256Digest::new(),
            blake3_hasher: if options.hash_blake3_enabled {
                Some(blake3::Hasher::new())
            } else {
                None
            },
            xxh3_hasher: if options.hash_xxhash3_enabled {
                Some(xxhash_rust::xxh3::Xxh3::new())
            } else {
                None
            },
            xxh128_hasher: if options.hash_xxhash128_enabled {
                Some(xxhash_rust::xxh3::Xxh3::new())
            } else {
                None
            },
            bytes_processed: 0,
        }
    }

    /// Process data block (corresponds to VB.NET Propagate method)
    pub fn propagate(&mut self, data: &[u8]) {
        use sha1::Digest as Sha1Digest;
        use sha2::Digest as Sha256Digest;

        self.sha1_hasher.update(data);
        self.md5_hasher.consume(data);
        Sha256Digest::update(&mut self.sha256_hasher, data);

        if let Some(ref mut hasher) = self.blake3_hasher {
            hasher.update(data);
        }

        if let Some(ref mut hasher) = self.xxh3_hasher {
            hasher.update(data);
        }

        if let Some(ref mut hasher) = self.xxh128_hasher {
            hasher.update(data);
        }

        self.bytes_processed += data.len() as u64;
    }

    /// Complete final processing (corresponds to VB.NET ProcessFinalBlock method)
    pub fn process_final_block(&mut self) {
        // All hashers complete final processing when finalize is called
    }

    /// Get SHA1 value
    pub fn sha1_value(&self) -> String {
        use sha1::Digest;
        let hasher = self.sha1_hasher.clone();
        format!("{:X}", hasher.finalize())
    }

    /// Get MD5 value
    pub fn md5_value(&self) -> String {
        format!("{:X}", self.md5_hasher.clone().compute())
    }

    /// Get SHA256 value
    pub fn sha256_value(&self) -> String {
        use sha2::Digest;
        let hasher = self.sha256_hasher.clone();
        format!("{:X}", hasher.finalize())
    }

    /// Get BLAKE3 value
    pub fn blake3_value(&self) -> Option<String> {
        self.blake3_hasher
            .as_ref()
            .map(|hasher| hex::encode_upper(hasher.clone().finalize().as_bytes()))
    }

    /// Get XxHash3 value
    pub fn xxhash3_value(&self) -> Option<String> {
        self.xxh3_hasher
            .as_ref()
            .map(|hasher| format!("{:X}", hasher.clone().digest()))
    }

    /// Get XxHash128 value
    pub fn xxhash128_value(&self) -> Option<String> {
        self.xxh128_hasher
            .as_ref()
            .map(|hasher| format!("{:X}", hasher.clone().digest128()))
    }

    /// Get filtered hash map based on WriteOptions (LTFSCopyGUI compatible keys)
    pub fn get_enabled_hashes(&self, options: &WriteOptions) -> HashMap<String, String> {
        let mut hashes = HashMap::new();

        if options.hash_sha1_enabled {
            hashes.insert("ltfs.hash.sha1sum".to_string(), self.sha1_value());
        }

        if options.hash_md5_enabled {
            hashes.insert("ltfs.hash.md5sum".to_string(), self.md5_value());
        }

        // SHA256 is always included when hash_on_write is enabled
        hashes.insert("ltfs.hash.sha256sum".to_string(), self.sha256_value());

        if options.hash_blake3_enabled {
            if let Some(blake3) = self.blake3_value() {
                hashes.insert("ltfs.hash.blake3sum".to_string(), blake3);
            }
        }

        if options.hash_xxhash3_enabled {
            if let Some(xxh3) = self.xxhash3_value() {
                hashes.insert("ltfs.hash.xxhash3sum".to_string(), xxh3);
            }
        }

        if options.hash_xxhash128_enabled {
            if let Some(xxh128) = self.xxhash128_value() {
                hashes.insert("ltfs.hash.xxhash128sum".to_string(), xxh128);
            }
        }

        hashes
    }
}
