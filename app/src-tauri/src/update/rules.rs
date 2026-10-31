//! What may be offered and downloaded: the version rule, the download address rule, and the
//! constants that name the release feed and its signing key.

use semver::Version;
use url::Url;

/// GitHub's `latest` never points at a pre-release, so this feed carries stable releases only.
pub const ENDPOINT: &str = "https://github.com/iAmChumby/canvas-rgb/releases/latest/download/latest.json";

/// The release signing key (minisign public key, as `tauri signer generate` writes it). The
/// private half is the `UPDATE_SIGNING_KEY` Actions secret. The service checks installers against
/// the same constant. `plugins.updater.pubkey` in `tauri.conf.json` repeats it for the plugin's own
/// config; a test keeps the two equal.
pub const PUBKEY: &str = canvas_proto::consts::UPDATE_PUBKEY;

/// The manifest key of our one build (`platforms` in `latest.json`).
pub const TARGET: &str = "windows-x86_64";

/// Hosts a release download may come from: the release page and GitHub's asset hosts.
pub const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// A release is an update when it is strictly newer than the running version and is not a
/// pre-release. Build metadata (`+abc`) never counts toward the order.
pub fn is_update(current: &Version, release: &Version) -> bool {
    release.pre.is_empty() && core(release) > core(current)
}

fn core(v: &Version) -> Version {
    Version {
        build: semver::BuildMetadata::EMPTY,
        ..v.clone()
    }
}

/// Where a download may come from. Production: `https` on [`ALLOWED_HOSTS`], default port.
/// The tests widen it to a loopback server.
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    pub https_only: bool,
    pub hosts: Vec<String>,
}

impl DownloadPolicy {
    pub fn production() -> Self {
        DownloadPolicy {
            https_only: true,
            hosts: ALLOWED_HOSTS.iter().map(|h| h.to_string()).collect(),
        }
    }

    /// `Err` names why the address is refused.
    pub fn check(&self, url: &Url) -> Result<(), String> {
        if self.https_only {
            if url.scheme() != "https" {
                return Err(format!("the download address is not https: {url}"));
            }
            if url.port().is_some() {
                return Err(format!("the download address names a port: {url}"));
            }
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!("the download address carries a login: {url}"));
        }
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if self.hosts.contains(&host) {
            Ok(())
        } else {
            Err(format!("the download host {host:?} is not on the list"))
        }
    }
}

/// The installer's file name for a version: `canvas-rgb-setup-0.1.1.exe`. Only the three numbers
/// are used, so nothing in a version string can reach the path.
pub fn installer_file_name(version: &Version) -> String {
    format!("canvas-rgb-setup-{}.{}.{}.exe", version.major, version.minor, version.patch)
}
