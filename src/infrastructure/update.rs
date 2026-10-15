//! Update adapters: install detection, the signed manifest over HTTPS, the
//! verified installer download and running it after Loom quits.
//!
//! Packaged builds (`LOOM_UPDATES=1` at compile time, set by
//! `scripts/package`) check `releases/latest/download/update-<platform>.json`.
//! `LOOM_UPDATE_FEED=<base url>` points any build at another feed for testing.
use crate::features::update::{Asset, InstallKind, Platform, Release, parse_release};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const FEED: &str = "https://github.com/lius-new/loom/releases/latest/download";
/// Raw Ed25519 public key; `node scripts/package/package.mjs keygen` makes
/// the pair. The private key is the `LOOM_UPDATE_SIGNING_KEY` release secret.
const PUBLIC_KEY: &str = "c9t/ZmkmU2jqqw11/GVtbxY44Oor7YZrBFsU4QqfF8k=";
/// Manifests are a few hundred bytes; anything larger is not one.
const MANIFEST_LIMIT: u64 = 64 * 1024;
const INSTALLER_LIMIT: u64 = 1024 * 1024 * 1024;

static PENDING_INSTALLER: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn platform() -> Option<Platform> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(Platform::WindowsX64)
    } else if cfg!(target_os = "macos") {
        Some(Platform::MacUniversal)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(Platform::LinuxX64)
    } else {
        None
    }
}

fn feed() -> Option<String> {
    match std::env::var("LOOM_UPDATE_FEED") {
        Ok(url) if !url.trim().is_empty() => Some(url.trim().trim_end_matches('/').to_owned()),
        _ => (option_env!("LOOM_UPDATES") == Some("1")).then(|| FEED.to_owned()),
    }
}

/// How this copy was installed; `None` when it does not check for updates.
pub fn install_kind() -> Option<InstallKind> {
    feed()?;
    platform()?;
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(if cfg!(windows) {
        // Inno Setup leaves its uninstaller next to the installed files.
        if dir.join("unins000.exe").is_file() {
            InstallKind::WindowsSetup
        } else {
            InstallKind::WindowsPortable
        }
    } else if cfg!(target_os = "macos") {
        InstallKind::MacApp
    } else if std::env::var_os("APPIMAGE").is_some() {
        InstallKind::LinuxAppImage
    } else if exe.starts_with("/opt/loom") {
        InstallKind::LinuxDeb
    } else {
        InstallKind::LinuxTarball
    })
}

/// A test feed served from this machine may use plain HTTP.
fn is_loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

fn agent(url: &str) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("Loom/", env!("CARGO_PKG_VERSION")))
        // Honors HTTPS_PROXY and friends, except for a loopback test feed.
        .try_proxy_from_env(!is_loopback(url))
        .https_only(!is_loopback(url))
        .build()
}

fn get(url: &str) -> Result<ureq::Response, String> {
    agent(url).get(url).call().map_err(|error| match error {
        ureq::Error::Status(code, _) => format!("the server returned HTTP {code}"),
        error => error.to_string(),
    })
}

/// Fetch, verify and parse this platform's manifest.
pub fn check(platform: Platform) -> Result<Release, String> {
    let feed = feed().ok_or("updates are disabled in this build")?;
    let url = format!("{feed}/update-{}.json", platform.id());
    let mut body = Vec::new();
    get(&url)?
        .into_reader()
        .take(MANIFEST_LIMIT)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    let payload = verified_payload(&body, PUBLIC_KEY)?;
    parse_release(&payload, platform)
}

/// The signed payload bytes of a manifest envelope.
fn verified_payload(envelope: &[u8], public_key: &str) -> Result<Vec<u8>, String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        payload: String,
        signature: String,
    }
    let envelope: Envelope =
        serde_json::from_slice(envelope).map_err(|error| format!("invalid manifest: {error}"))?;
    let decode = |text: &str| BASE64.decode(text).map_err(|_| "invalid manifest encoding");
    let payload = decode(&envelope.payload)?;
    let signature = decode(&envelope.signature)?;
    let key = decode(public_key)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(&payload, &signature)
        .map_err(|_| "the update manifest signature is invalid".to_owned())?;
    Ok(payload)
}

/// Download `asset` into the update folder and check its size and SHA-256.
/// `progress` receives the bytes received so far, about once per percent.
pub fn download(asset: &Asset, mut progress: impl FnMut(u64)) -> Result<PathBuf, String> {
    let name = Path::new(&asset.name)
        .file_name()
        .filter(|name| *name == asset.name.as_str())
        .ok_or("invalid installer name")?;
    if asset.size > INSTALLER_LIMIT {
        return Err("the installer is too large".to_owned());
    }
    let dir = std::env::temp_dir().join("loom-update");
    // Earlier downloads are never needed again.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let target = dir.join(name);
    let partial = target.with_extension("part");

    let io = |error: std::io::Error| error.to_string();
    let mut reader = get(&asset.url)?.into_reader().take(asset.size + 1);
    let mut file = std::fs::File::create(&partial).map_err(io)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; 64 * 1024];
    let mut received = 0u64;
    let step = (asset.size / 100).max(1);
    let mut reported = 0u64;
    loop {
        let read = reader.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(io)?;
        digest.update(&buffer[..read]);
        received += read as u64;
        if received - reported >= step {
            reported = received;
            progress(received);
        }
    }
    file.sync_all().map_err(io)?;
    drop(file);

    let actual = hex(digest.finish().as_ref());
    if received != asset.size || actual != asset.sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err("the downloaded installer does not match the release".to_owned());
    }
    std::fs::rename(&partial, &target).map_err(io)?;
    progress(received);
    Ok(target)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Run `installer` once Loom quits (see `run_pending_installer`).
pub fn install_on_exit(installer: PathBuf) {
    *PENDING_INSTALLER.lock().unwrap() = Some(installer);
}

/// Start the installer recorded by `install_on_exit`, after the window has
/// closed. The installer closes any other Loom instance, replaces the
/// files and starts Loom again (`/relaunch=1`, see `installer.iss`).
pub fn run_pending_installer() {
    let Some(installer) = PENDING_INSTALLER.lock().unwrap().take() else {
        return;
    };
    let _ = std::process::Command::new(installer)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/relaunch=1",
        ])
        .spawn();
}

/// Open a release page in the default browser.
pub fn open_page(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = |text: &str| {
            std::ffi::OsStr::new(text)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<u16>>()
        };
        let (verb, target) = (wide("open"), wide(url));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success with a value above 32.
        if result as usize > 32 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed test key pair (RFC 8032 test 1), not the release key.
    const TEST_PUBLIC: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
    const TEST_SIGNATURE_OF_EMPTY: &str =
        "5VZDAMNgrHKQhuLMgG6CioSHfx645dl02HPgZSJJAVVfuIIVkKM7rMYeOXAc+bRr0lv18FlbviRlUUFDjnoQCw==";

    fn envelope(payload: &[u8], signature: &str) -> Vec<u8> {
        format!(
            r#"{{"payload":"{}","signature":"{signature}"}}"#,
            BASE64.encode(payload)
        )
        .into_bytes()
    }

    #[test]
    fn accepts_only_payloads_signed_by_the_key() {
        assert_eq!(
            verified_payload(&envelope(b"", TEST_SIGNATURE_OF_EMPTY), TEST_PUBLIC).unwrap(),
            b""
        );
        assert!(verified_payload(&envelope(b"x", TEST_SIGNATURE_OF_EMPTY), TEST_PUBLIC).is_err());
        assert!(
            verified_payload(&envelope(b"", TEST_SIGNATURE_OF_EMPTY), PUBLIC_KEY).is_err(),
            "the release key must not verify another key's signature"
        );
        assert!(verified_payload(b"not json", TEST_PUBLIC).is_err());
    }

    #[test]
    fn release_key_is_a_raw_ed25519_key() {
        assert_eq!(BASE64.decode(PUBLIC_KEY).unwrap().len(), 32);
    }

    /// Reads the real feed, or `LOOM_UPDATE_FEED`:
    /// `cargo test live_feed -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_feed_is_signed_by_the_release_key() {
        let platform = platform().expect("a released platform");
        let feed = feed().unwrap_or_else(|| FEED.to_owned());
        let url = format!("{feed}/update-{}.json", platform.id());
        let mut body = Vec::new();
        get(&url)
            .unwrap()
            .into_reader()
            .read_to_end(&mut body)
            .unwrap();
        let payload = verified_payload(&body, PUBLIC_KEY).unwrap();
        let release = parse_release(&payload, platform).unwrap();
        println!("{url}: {} {:?}", release.version, release.assets.keys());
    }

    #[test]
    fn only_loopback_feeds_may_use_http() {
        assert!(is_loopback("http://127.0.0.1:8080/update.json"));
        assert!(is_loopback("http://localhost:9000"));
        assert!(!is_loopback("http://127.0.0.1.example.com/"));
        assert!(!is_loopback("http://example.com/"));
    }

    #[test]
    fn hashes_print_as_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }
}
