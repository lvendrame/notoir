//! Updates (docs/specs/31 §Updates): a JSON manifest signed with minisign
//! (Ed25519) is fetched over HTTPS; nothing is offered unless its signature
//! verifies against the public key built into the binary and its version is
//! greater than the running one. Installers are downloaded to the cache,
//! checked against the SHA-256 in the signed manifest, and only handed to
//! the platform installer after Notoir has exited normally.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Where the manifest lives and the key it must be signed with. Release
/// builds set both at compile time (`NOTOIR_UPDATE_URL`,
/// `NOTOIR_UPDATE_PUBKEY`); without them the updater is unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    pub manifest_url: String,
    /// The minisign public key (the base64 line of `minisign.pub`).
    pub public_key: String,
}

impl UpdateConfig {
    pub fn built_in() -> Option<UpdateConfig> {
        let url = option_env!("NOTOIR_UPDATE_URL")?.trim();
        let key = option_env!("NOTOIR_UPDATE_PUBKEY")?.trim();
        (!url.is_empty() && !key.is_empty()).then(|| UpdateConfig {
            manifest_url: url.to_owned(),
            public_key: key.to_owned(),
        })
    }
}

/// The installer kind this build updates with: `macos` (.dmg),
/// `windows` (.msi), `linux-appimage`; `None` for Linux distro packages
/// (their package manager updates them) and other systems.
pub fn platform_key() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("macos")
    } else if cfg!(target_os = "windows") {
        Some("windows")
    } else if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some() {
        Some("linux-appimage")
    } else {
        None
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestFile {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    artifacts: HashMap<String, Artifact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
}

/// An update newer than the running version, for this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub notes: Option<String>,
    pub artifact: Artifact,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("network: {0}")]
    Network(String),
    #[error("signature: {0}")]
    Signature(String),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("download: {0}")]
    Download(String),
}

/// Verifies `manifest` against `signature` (the `.minisig` text) and, when
/// valid, returns the offer for `platform` if it is newer than `current`.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &str,
    public_key: &str,
    current: &str,
    platform: &str,
) -> Result<Option<Offer>, UpdateError> {
    let sig = |e: minisign_verify::Error| UpdateError::Signature(e.to_string());
    let key = PublicKey::from_base64(public_key).map_err(sig)?;
    let signature = Signature::decode(signature).map_err(sig)?;
    // Accept both prehashed (current minisign) and legacy signatures.
    key.verify(manifest, &signature, true).map_err(sig)?;
    let file: ManifestFile =
        serde_json::from_slice(manifest).map_err(|e| UpdateError::Manifest(e.to_string()))?;
    let version = semver::Version::parse(file.version.trim())
        .map_err(|e| UpdateError::Manifest(e.to_string()))?;
    let running =
        semver::Version::parse(current).map_err(|e| UpdateError::Manifest(e.to_string()))?;
    if version <= running {
        return Ok(None);
    }
    let Some(artifact) = file.artifacts.get(platform).cloned() else {
        return Ok(None);
    };
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::Manifest("bad sha256".into()));
    }
    Ok(Some(Offer {
        version: version.to_string(),
        notes: file.notes,
        artifact,
    }))
}

const MANIFEST_LIMIT: u64 = 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(concat!("Notoir-updater/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let net = |e: ureq::Error| UpdateError::Network(e.to_string());
    let mut resp = agent.get(url).call().map_err(net)?;
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(net)
}

/// Fetches and verifies the manifest (`<url>` and `<url>.minisig`).
pub fn check(
    cfg: &UpdateConfig,
    current: &str,
    platform: &str,
) -> Result<Option<Offer>, UpdateError> {
    let agent = agent();
    let manifest = get_bytes(&agent, &cfg.manifest_url, MANIFEST_LIMIT)?;
    let sig = get_bytes(&agent, &format!("{}.minisig", cfg.manifest_url), 64 * 1024)?;
    let sig = String::from_utf8(sig).map_err(|e| UpdateError::Signature(e.to_string()))?;
    verify_manifest(&manifest, &sig, &cfg.public_key, current, platform)
}

/// Downloads the installer into `dir` and checks its SHA-256; a mismatch
/// deletes the file.
pub fn download(offer: &Offer, dir: &Path) -> Result<PathBuf, UpdateError> {
    let fail = |e: String| UpdateError::Download(e);
    std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    let name = offer
        .artifact
        .url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && !n.contains(['\\', ':']) && *n != "." && *n != "..")
        .unwrap_or("notoir-update")
        .to_owned();
    let path = dir.join(name);
    let partial = path.with_extension("part");
    let mut resp = agent()
        .get(&offer.artifact.url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let mut reader = resp
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024 * 1024)
        .reader();
    let mut file = std::fs::File::create(&partial).map_err(|e| fail(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| fail(e.to_string()))?;
    }
    file.sync_all().map_err(|e| fail(e.to_string()))?;
    drop(file);
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !got.eq_ignore_ascii_case(&offer.artifact.sha256) {
        let _ = std::fs::remove_file(&partial);
        return Err(fail("checksum mismatch".into()));
    }
    std::fs::rename(&partial, &path).map_err(|e| fail(e.to_string()))?;
    Ok(path)
}

/// Hands a verified installer to the platform after Notoir exits: the disk
/// image / MSI opens with the OS (the user completes the install); an
/// AppImage replaces the running one, effective on the next launch.
pub fn hand_off(installer: &Path) -> std::io::Result<()> {
    if let Some(current) = std::env::var_os("APPIMAGE").filter(|_| cfg!(target_os = "linux")) {
        let current = PathBuf::from(current);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(installer, std::fs::Permissions::from_mode(0o755))?;
        }
        return std::fs::rename(installer, current);
    }
    open::that_detached(installer)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

    /// A minisign key pair for tests: (public key line, signer).
    pub fn test_key(seed: u8) -> (String, impl Fn(&[u8]) -> String) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let key_id = [seed; 8];
        let mut public = b"Ed".to_vec();
        public.extend(key_id);
        public.extend(sk.verifying_key().as_bytes());
        let sign = move |msg: &[u8]| {
            let sig = sk.sign(msg).to_bytes();
            let mut line = b"Ed".to_vec();
            line.extend(key_id);
            line.extend(sig);
            let trusted = "timestamp:0\tfile:update.json";
            let mut global = sig.to_vec();
            global.extend(trusted.as_bytes());
            let global = sk.sign(&global).to_bytes();
            format!(
                "untrusted comment: test\n{}\ntrusted comment: {trusted}\n{}\n",
                B64.encode(line),
                B64.encode(global)
            )
        };
        (B64.encode(public), sign)
    }

    fn manifest(version: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","notes":"Fixes","artifacts":{{"macos":{{"url":"https://example.invalid/Notoir.dmg","sha256":"{}"}}}}}}"#,
            "ab".repeat(32)
        )
        .into_bytes()
    }

    #[test]
    fn signed_newer_manifests_are_offered() {
        let (key, sign) = test_key(1);
        let m = manifest("1.2.0");
        let offer = verify_manifest(&m, &sign(&m), &key, "1.1.9", "macos").unwrap();
        assert_eq!(offer.as_ref().map(|o| o.version.as_str()), Some("1.2.0"));
        // Not newer, or nothing for this platform: no offer.
        assert_eq!(
            verify_manifest(&m, &sign(&m), &key, "1.2.0", "macos"),
            Ok(None)
        );
        assert_eq!(
            verify_manifest(&m, &sign(&m), &key, "0.1.0", "windows"),
            Ok(None)
        );
    }

    #[test]
    fn bad_signatures_are_rejected() {
        let (key, sign) = test_key(1);
        let (_, other) = test_key(2);
        let m = manifest("9.0.0");
        // Tampered content.
        let mut tampered = m.clone();
        tampered[12] ^= 1;
        for (bytes, sig) in [
            (&tampered, sign(&m)),
            (&m, other(&m)),
            (&m, "garbage".to_owned()),
        ] {
            assert!(
                matches!(
                    verify_manifest(bytes, &sig, &key, "0.1.0", "macos"),
                    Err(UpdateError::Signature(_))
                ),
                "{sig}"
            );
        }
    }
}
