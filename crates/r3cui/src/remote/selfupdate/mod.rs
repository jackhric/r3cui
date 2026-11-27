//! r3cui's own updates: what GitHub says the latest release is, fetching its archive and checking it. Putting the
//! archive on the remote is `remote::rest`.

use anyhow::{anyhow, bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub(crate) const REPO_PATH: &str = "github.com/jackhric/r3cui";
/// The file in a release that the remote installs: `r3cui-<version>.tar.gz`, as `packaging/pack.sh` names it.
pub(crate) fn asset_name(version: &Version) -> String {
    format!("r3cui-{version}.tar.gz")
}
/// Overrides `LATEST_URL`: a stand-in for GitHub's API in tests and on the desktop.
pub(crate) const LATEST_URL_ENV: &str = "UC_RELEASES_URL";
const LATEST_URL: &str = "https://api.github.com/repos/jackhric/r3cui/releases/latest";
const USER_AGENT: &str = "r3cui";
const DIGEST_PREFIX: &str = "sha256:";
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const CHUNK: usize = 64 << 10;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Release {
    pub(crate) version: Version,
    /// The release's page, which the update screen shows as a QR code.
    pub(crate) page_url: String,
    pub(crate) published_at: String,
    pub(crate) size: u64,
    pub(crate) download_url: String,
    /// Lower-case hex.
    pub(crate) sha256: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

/// The version a tag names: `v0.2.0` or `0.2.0`.
pub(crate) fn tag_version(tag: &str) -> Result<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).with_context(|| format!("release tag {tag:?} is not a version"))
}

pub(crate) fn parse_release(json: &str) -> Result<Release> {
    let r: ReleaseJson = serde_json::from_str(json).context("the release is not what GitHub's API describes")?;
    if r.draft || r.prerelease {
        bail!("the latest release is a {}", if r.draft { "draft" } else { "prerelease" });
    }
    let version = tag_version(&r.tag_name)?;
    let name = asset_name(&version);
    let asset = r.assets.into_iter().find(|a| a.name == name).ok_or_else(|| anyhow!("the release has no {name}"))?;
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix(DIGEST_PREFIX))
        .ok_or_else(|| anyhow!("{name} has no checksum to check it against"))?
        .to_ascii_lowercase();
    Ok(Release { version, page_url: r.html_url, published_at: r.published_at, size: asset.size, download_url: asset.browser_download_url, sha256 })
}

/// The commit this build came from, 7 hex digits (`scripts/version.sh --commit`); empty outside a git checkout.
pub(crate) const COMMIT: &str = env!("R3CUI_COMMIT");

/// The version this build is, from `scripts/version.sh`: a release's own, or one with the commit after a `+`.
pub(crate) fn current_version() -> Version {
    Version::parse(env!("R3CUI_VERSION")).expect("scripts/version.sh gives semver")
}

/// The latest release, and whether it is newer than `running`. The commit after a `+` doesn't count: a build of a
/// release's own version is not behind it.
pub(crate) fn newer(release: Release, running: &Version) -> Option<Release> {
    release.version.cmp_precedence(running).is_gt().then_some(release)
}

fn latest_url() -> String {
    std::env::var(LATEST_URL_ENV).unwrap_or_else(|_| LATEST_URL.to_string())
}

/// Blocking: call from `spawn_blocking`.
pub(crate) fn fetch_latest() -> Result<Release> {
    fetch_latest_from(&latest_url())
}

fn fetch_latest_from(url: &str) -> Result<Release> {
    let body = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(CHECK_TIMEOUT)
        .call()
        .with_context(|| format!("asking {url} for the latest release"))?
        .into_string()?;
    parse_release(&body)
}

/// Writes the release's archive to `dest`, reporting (bytes so far, total) as it goes, and removes it again if its
/// checksum is not the one GitHub published. Blocking.
pub(crate) fn download(release: &Release, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let result = fetch_to(&release.download_url, dest, release.size, &mut progress).and_then(|sum| {
        if sum.eq_ignore_ascii_case(&release.sha256) {
            Ok(())
        } else {
            Err(anyhow!("the download is damaged (checksum {sum}, expected {})", release.sha256))
        }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn fetch_to(url: &str, dest: &Path, expected: u64, progress: &mut impl FnMut(u64, u64)) -> Result<String> {
    let response = ureq::get(url).set("User-Agent", USER_AGENT).timeout(DOWNLOAD_TIMEOUT).call().with_context(|| format!("downloading {url}"))?;
    let total = response.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(expected);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(dest).with_context(|| format!("writing {}", dest.display()))?;
    let (mut hash, mut done, mut buf) = (Sha256::new(), 0u64, vec![0u8; CHUNK]);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.flush()?;
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Empties r3cui's own config and data folders (never the state folder, which the stock UI shares), leaving the
/// folders. What could not be removed is returned: the UI is on its way out, so that is no reason to stop.
pub(crate) fn wipe_own_files(folders: &[&Path]) -> Vec<String> {
    let mut failed = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let gone = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            if let Err(e) = gone {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
    }
    failed
}

#[cfg(test)]
pub(crate) mod fake_http;

#[cfg(test)]
mod tests {
    use super::fake_http::{Canned, FakeHttp};
    use super::*;
    use serde_json::json;

    fn release_json(tag: &str, digest: Option<&str>, download: &str) -> String {
        json!({
            "tag_name": tag, "html_url": format!("https://github.com/jackhric/r3cui/releases/tag/{tag}"), "published_at": "2026-11-20T09:00:00Z",
            "assets": [
                {"name": "other.zip", "size": 1, "browser_download_url": "https://example.invalid/other.zip"},
                {"name": asset_name(&tag_version(tag).unwrap()), "size": 11, "browser_download_url": download, "digest": digest},
            ],
        })
        .to_string()
    }

    const HELLO_SHA: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn tags_compare_as_semver() {
        let v = |s| tag_version(s).unwrap();
        assert!(v("v0.2.0") > v("0.1.9"));
        assert!(v("v0.10.0") > v("v0.9.0"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert_eq!(v("v1.2.3"), Version::new(1, 2, 3));
        assert!(tag_version("nightly").is_err());
    }

    #[test]
    fn a_release_is_read_for_its_archive_and_checksum() {
        let r = parse_release(&release_json("v0.2.0", Some(&format!("sha256:{}", HELLO_SHA.to_uppercase())), "https://x/r3cui-0.2.0.tar.gz")).unwrap();
        assert_eq!((r.version.to_string().as_str(), r.size, r.sha256.as_str()), ("0.2.0", 11, HELLO_SHA));
        assert_eq!(r.page_url, "https://github.com/jackhric/r3cui/releases/tag/v0.2.0");
        assert!(newer(r.clone(), &Version::new(0, 1, 0)).is_some());
        assert!(newer(r.clone(), &Version::new(0, 2, 0)).is_none());
        assert!(newer(r.clone(), &Version::new(0, 3, 0)).is_none());
        assert!(newer(r.clone(), &Version::parse("0.2.0+g1eb1474").unwrap()).is_none(), "a build of the release itself");
        assert!(newer(r, &Version::parse("0.1.0+g1eb1474").unwrap()).is_some());
    }

    #[test]
    fn this_build_knows_its_version_and_the_archive_is_named_for_it() {
        assert_eq!(current_version().to_string(), env!("R3CUI_VERSION"));
        assert_eq!(asset_name(&Version::new(0, 2, 0)), "r3cui-0.2.0.tar.gz");
    }

    #[test]
    fn a_release_that_cannot_be_installed_is_an_error() {
        assert!(parse_release(&release_json("v0.2.0", None, "u")).unwrap_err().to_string().contains("checksum"));
        assert!(parse_release(r#"{"tag_name":"v1.0.0","html_url":"u","assets":[]}"#).unwrap_err().to_string().contains("r3cui-1.0.0.tar.gz"));
        assert!(parse_release(r#"{"tag_name":"v1.0.0","html_url":"u","prerelease":true,"assets":[]}"#).is_err());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn the_latest_release_is_fetched_with_a_user_agent() {
        let http = FakeHttp::start(vec![Canned::ok("/latest", release_json("v0.3.0", Some("sha256:ab"), "u"))]);
        let r = fetch_latest_from(&http.url("/latest")).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 0));
        let seen = http.requests();
        assert_eq!(seen[0].header("user-agent"), Some(USER_AGENT));
        assert!(fetch_latest_from(&http.url("/missing")).is_err());
    }

    #[test]
    fn wiping_empties_the_folders_and_keeps_them() {
        let root = std::env::temp_dir().join(format!("r3cui-wipe-{}", std::process::id()));
        let (config, data, state) = (root.join("config"), root.join("data"), root.join("state"));
        for dir in [&config, &data, &state] {
            std::fs::create_dir_all(dir.join("nested")).unwrap();
            std::fs::write(dir.join("file"), "x").unwrap();
            std::fs::write(dir.join("nested/file"), "x").unwrap();
        }
        assert!(wipe_own_files(&[&config, &data, &root.join("missing")]).is_empty());
        assert_eq!(std::fs::read_dir(&config).unwrap().count() + std::fs::read_dir(&data).unwrap().count(), 0);
        assert!(state.join("nested/file").exists(), "the state folder is not r3cui's to empty");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_download_is_checked_against_the_published_checksum() {
        let http = FakeHttp::start(vec![Canned::ok("/a.tar.gz", "hello world")]);
        let dir = std::env::temp_dir().join(format!("r3cui-dl-{}", std::process::id()));
        let dest = dir.join("a.tar.gz");
        let release = Release {
            version: Version::new(0, 2, 0),
            page_url: String::new(),
            published_at: String::new(),
            size: 11,
            download_url: http.url("/a.tar.gz"),
            sha256: HELLO_SHA.into(),
        };
        let mut last = (0, 0);
        download(&release, &dest, |done, total| last = (done, total)).unwrap();
        assert_eq!((std::fs::read(&dest).unwrap(), last), (b"hello world".to_vec(), (11, 11)));

        let tampered = Release { sha256: "0".repeat(64), ..release };
        let err = download(&tampered, &dest, |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("damaged"), "{err}");
        assert!(!dest.exists(), "a damaged archive is not kept");
        let _ = std::fs::remove_dir_all(dir);
    }
}
