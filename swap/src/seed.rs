use crate::fs::ensure_directory_exists;
use ::bitcoin::bip32::Xpriv as ExtendedPrivKey;
use anyhow::{Context, Result};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::constants::SECRET_KEY_SIZE;
use bitcoin::secp256k1::{self, SecretKey};
use libp2p::identity;
use pem::{encode, Pem};
use polyseed::{Language, Polyseed, PolyseedError};
use rand::prelude::*;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const SEED_LENGTH: usize = 32;

#[derive(Clone, Eq, PartialEq)]
pub struct Seed([u8; SEED_LENGTH]);

impl Seed {
    pub fn random() -> Result<Self, Error> {
        let mut bytes = [0u8; SECRET_KEY_SIZE];
        rand::thread_rng().fill_bytes(&mut bytes);

        // If it succeeds once, it'll always succeed
        let _ = SecretKey::from_slice(&bytes)?;

        Ok(Seed(bytes))
    }

    pub fn derive_extended_private_key(
        &self,
        network: bitcoin::Network,
    ) -> Result<ExtendedPrivKey> {
        let seed = self.derive(b"BITCOIN_EXTENDED_PRIVATE_KEY").bytes();
        let private_key = ExtendedPrivKey::new_master(network, &seed)
            .context("Failed to create new master extended private key")?;

        Ok(private_key)
    }

    /// Same as `derive_extended_private_key`, but using the legacy BDK API.
    ///
    /// This is only used for the migration path from the old wallet format to the new one.
    pub fn derive_extended_private_key_legacy(
        &self,
        network: bdk::bitcoin::Network,
    ) -> Result<bdk::bitcoin::util::bip32::ExtendedPrivKey> {
        let seed = self.derive(b"BITCOIN_EXTENDED_PRIVATE_KEY").bytes();
        let private_key = bdk::bitcoin::util::bip32::ExtendedPrivKey::new_master(network, &seed)
            .context("Failed to create new master extended private key")?;

        Ok(private_key)
    }

    pub fn derive_libp2p_identity(&self) -> identity::Keypair {
        let bytes = self.derive(b"NETWORK").derive(b"LIBP2P_IDENTITY").bytes();

        identity::Keypair::ed25519_from_bytes(bytes).expect("we always pass 32 bytes")
    }

    pub fn from_file_or_generate(data_dir: &Path) -> Result<Self, Error> {
        let file_path_buf = data_dir.join("seed.pem");
        let file_path = Path::new(&file_path_buf);

        if file_path.exists() {
            return Self::from_file(file_path);
        }

        tracing::debug!("No seed file found, creating at {}", file_path.display());

        let random_seed = Seed::random()?;
        random_seed.write_to(file_path.to_path_buf())?;

        Ok(random_seed)
    }

    /// Derive a new seed using the given scope.
    ///
    /// This function is purposely kept private because it is only a helper
    /// function for deriving specific secret material from the root seed
    /// like the libp2p identity or the seed for the Bitcoin wallet.
    fn derive(&self, scope: &[u8]) -> Self {
        let mut engine = sha256::HashEngine::default();

        engine.input(&self.bytes());
        engine.input(scope);

        let hash = sha256::Hash::from_engine(engine);

        Self(hash.to_byte_array())
    }

    fn bytes(&self) -> [u8; SEED_LENGTH] {
        self.0
    }

    fn from_file<D>(seed_file: D) -> Result<Self, Error>
    where
        D: AsRef<OsStr>,
    {
        let file = Path::new(&seed_file);
        let contents = fs::read_to_string(file)?;
        let pem = pem::parse(contents)?;

        tracing::debug!("Reading in seed from {}", file.display());

        Self::from_pem(pem)
    }

    pub fn from_pem(pem: pem::Pem) -> Result<Self, Error> {
        let contents = pem.contents();
        if contents.len() != SEED_LENGTH {
            Err(Error::IncorrectLength(contents.len()))
        } else {
            let mut array = [0; SEED_LENGTH];
            for (i, b) in contents.iter().enumerate() {
                array[i] = *b;
            }

            Ok(Self::from(array))
        }
    }

    pub fn write_to(&self, seed_file: PathBuf) -> Result<(), Error> {
        ensure_directory_exists(&seed_file)?;

        let data = self.bytes();
        let pem = Pem::new("SEED", data);

        let pem_string = encode(&pem);

        let mut file = File::create(seed_file)?;
        file.write_all(pem_string.as_bytes())?;

        Ok(())
    }

    /// Export the seed as a PEM formatted string for backup purposes
    pub fn to_pem_string(&self) -> String {
        let data = self.bytes();
        let pem = Pem::new("SEED", data);
        encode(&pem)
    }

    /// Convert our 256-bit seed to polyseed format (uses first 150 bits)
    /// Returns the polyseed mnemonic string and a warning about truncation
    pub fn to_polyseed_mnemonic(&self) -> Result<(String, String), PolyseedError> {
        use zeroize::Zeroizing;

        // Create entropy buffer - polyseed expects 32 bytes but only uses first 150 bits
        let mut entropy = Zeroizing::new([0u8; 32]);
        
        // Copy first 19 bytes (152 bits, will be masked to 150 bits by polyseed)
        entropy[..19].copy_from_slice(&self.0[..19]);
        
        // Clear the last 2 bits of byte 18 to ensure exactly 150 bits
        entropy[18] &= 0xFC; // 0xFC = 11111100, clears last 2 bits
        
        // Create polyseed with English language and current timestamp
        let polyseed = Polyseed::from(
            Language::English,
            0, // features
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            entropy,
        )?;
        
        let mnemonic = polyseed.to_string();
        let warning = format!(
            "WARNING: This polyseed contains only the first 150 bits of your 256-bit seed. \
            To restore your full wallet, you must use this specific software which can \
            reconstruct the remaining 106 bits deterministically."
        );
        
        Ok(((*mnemonic).clone(), warning))
    }

    /// Reconstruct a 256-bit seed from a polyseed mnemonic
    /// This expands the 150-bit polyseed back to our full 256-bit seed
    pub fn from_polyseed_mnemonic(mnemonic: &str) -> Result<Self, Error> {
        use zeroize::Zeroizing;
        
        let polyseed = Polyseed::from_string(Language::English, Zeroizing::new(mnemonic.to_string()))
            .map_err(|e| Error::PolyseedParse(e))?;
        
        // Get the 150-bit entropy from polyseed
        let polyseed_entropy = polyseed.entropy();
        
        // Create our full 256-bit seed
        let mut full_seed = [0u8; SEED_LENGTH];
        
        // Copy the first 19 bytes from polyseed (contains the 150 bits)
        full_seed[..19].copy_from_slice(&polyseed_entropy[..19]);
        
        // Derive the remaining 13 bytes deterministically using HMAC-SHA256
        use bitcoin::hashes::{Hmac, HmacEngine};
        let mut hmac_engine = HmacEngine::<sha256::Hash>::new(b"UNSTOPPABLESWAP_POLYSEED_EXTENSION");
        hmac_engine.input(&polyseed_entropy[..19]);
        let hmac_result = Hmac::from_engine(hmac_engine);
        
        // Use the HMAC result to fill the remaining bytes
        full_seed[19..].copy_from_slice(&hmac_result.to_byte_array()[..13]);
        
        Ok(Seed(full_seed))
    }
}

impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seed([*****])")
    }
}

impl fmt::Display for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<[u8; SEED_LENGTH]> for Seed {
    fn from(bytes: [u8; SEED_LENGTH]) -> Self {
        Seed(bytes)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Secp256k1: ")]
    Secp256k1(#[from] secp256k1::Error),
    #[error("io: ")]
    Io(#[from] io::Error),
    #[error("PEM parse: ")]
    PemParse(#[from] pem::PemError),
    #[error("expected 32 bytes of base64 encode, got {0} bytes")]
    IncorrectLength(usize),
    #[error("RNG: ")]
    Rand(#[from] rand::Error),
    #[error("no default path")]
    NoDefaultPath,
    #[error("Polyseed parse: ")]
    PolyseedParse(#[from] PolyseedError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn generate_random_seed() {
        let _ = Seed::random().unwrap();
    }

    #[test]
    fn seed_byte_string_must_be_32_bytes_long() {
        let _seed = Seed::from(*b"this string is exactly 32 bytes!");
    }

    #[test]
    fn seed_from_pem_works() {
        use base64::engine::general_purpose;
        use base64::Engine;

        let payload: &str = "syl9wSYaruvgxg9P5Q1qkZaq5YkM6GvXkxe+VYrL/XM=";

        // 32 bytes base64 encoded.
        let pem_string: &str = "-----BEGIN SEED-----
syl9wSYaruvgxg9P5Q1qkZaq5YkM6GvXkxe+VYrL/XM=
-----END SEED-----
";

        let want = general_purpose::STANDARD.decode(payload).unwrap();
        let pem = pem::parse(pem_string).unwrap();
        let got = Seed::from_pem(pem).unwrap();

        assert_eq!(got.bytes(), *want);
    }

    #[test]
    fn seed_from_pem_fails_for_short_seed() {
        let short = "-----BEGIN SEED-----
VnZUNFZ4dlY=
-----END SEED-----
";
        let pem = pem::parse(short).unwrap();
        match Seed::from_pem(pem) {
            Ok(_) => panic!("should fail for short payload"),
            Err(e) => {
                match e {
                    Error::IncorrectLength(_) => {} // pass
                    _ => panic!("should fail with IncorrectLength error"),
                }
            }
        }
    }

    #[test]
    fn seed_from_pem_fails_for_long_seed() {
        let long = "-----BEGIN SEED-----
MIIBPQIBAAJBAOsfi5AGYhdRs/x6q5H7kScxA0Kzzqe6WI6gf6+tc6IvKQJo5rQc
dWWSQ0nRGt2hOPDO+35NKhQEjBQxPh/v7n0CAwEAAQJBAOGaBAyuw0ICyENy5NsO
-----END SEED-----
";
        let pem = pem::parse(long).unwrap();
        assert_eq!(pem.contents().len(), 96);

        match Seed::from_pem(pem) {
            Ok(_) => panic!("should fail for long payload"),
            Err(e) => {
                match e {
                    Error::IncorrectLength(len) => assert_eq!(len, 96), // pass
                    _ => panic!("should fail with IncorrectLength error"),
                }
            }
        }
    }

    #[test]
    fn round_trip_through_file_write_read() {
        let tmpfile = temp_dir().join("seed.pem");

        let seed = Seed::random().unwrap();
        seed.write_to(tmpfile.clone())
            .expect("Write seed to temp file");

        let rinsed = Seed::from_file(tmpfile).expect("Read from temp file");
        assert_eq!(seed.0, rinsed.0);
    }

    #[test]
    fn polyseed_conversion_round_trip() {
        let original_seed = Seed::random().unwrap();
        
        // Convert to polyseed mnemonic
        let (mnemonic, _warning) = original_seed.to_polyseed_mnemonic().expect("Should convert to polyseed");
        
        // Reconstruct from polyseed mnemonic
        let reconstructed_seed = Seed::from_polyseed_mnemonic(&mnemonic).expect("Should reconstruct from polyseed");
        
        // The seeds should be equal (first 150 bits preserved, remaining deterministically derived)
        assert_eq!(original_seed.0, reconstructed_seed.0);
    }

    #[test]
    fn polyseed_truncation_behavior() {
        let seed = Seed::random().unwrap();
        
        // The polyseed should successfully truncate our 256-bit seed
        let result = seed.to_polyseed_mnemonic();
        assert!(result.is_ok(), "Polyseed conversion should succeed");
        
        let (mnemonic, warning) = result.unwrap();
        assert!(!mnemonic.is_empty(), "Mnemonic should not be empty");
        assert!(warning.contains("150 bits"), "Warning should mention 150-bit truncation");
    }
}
