//! What a release is, and how the updater reads one: version numbers, the
//! `releases/latest` JSON, the `SHA256SUMS` file, and the one place the
//! download host is allowed to be. No AppKit, no I/O: everything here is a
//! function of its arguments, and unit-tested as such.

use std::fmt;

/// The repository releases come from. `Source::github` and the asset
/// contract in the design doc are both derived from it.
pub const REPO: &str = "billsun9305/vitals";

/// The checksum asset's name.
pub const SUMS_NAME: &str = "SHA256SUMS";

/// A plain `MAJOR.MINOR.PATCH` version.
///
/// Pre-release suffixes are rejected on purpose: `releases/latest` never
/// returns a pre-release, and a bundle's `CFBundleShortVersionString` is
/// always plain, so a suffix anywhere means something is wrong. The derive
/// order of the fields is the comparison order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `"0.2.0"` or `"v0.2.0"`, surrounding whitespace ignored.
    pub fn parse(s: &str) -> Result<Version, UpdateError> {
        let text = s.trim();
        let body = text.strip_prefix('v').unwrap_or(text);
        let mut parts = body.split('.');
        let mut next = |what: &str| -> Result<u32, UpdateError> {
            parts
                .next()
                .ok_or_else(|| UpdateError::BadVersion(format!("{text:?}: missing {what}")))?
                .parse::<u32>()
                .map_err(|_| UpdateError::BadVersion(format!("{text:?}: {what} is not a number")))
        };
        let major = next("major")?;
        let minor = next("minor")?;
        let patch = next("patch")?;
        if parts.next().is_some() {
            return Err(UpdateError::BadVersion(format!(
                "{text:?}: more than three components"
            )));
        }
        Ok(Version {
            major,
            minor,
            patch,
        })
    }

    /// Parses the `MAJOR.MINOR.PATCH` prefix of a crate version, ignoring
    /// any `-suffix` (a pre-release build's `CARGO_PKG_VERSION`, e.g.
    /// `"0.2.0-beta.1"`). Falls back to `0.0.0` if even that fails — this
    /// is total, never panics, because it backs [`Version::current`].
    pub fn from_crate_version(s: &str) -> Version {
        let base = s.split('-').next().unwrap_or(s);
        Version::parse(base).unwrap_or(Version {
            major: 0,
            minor: 0,
            patch: 0,
        })
    }

    /// The version this binary was built as.
    ///
    /// A pre-release build (`CARGO_PKG_VERSION` like `"0.2.0-beta.1"`)
    /// reports its base version, `0.2.0` — it is not offered the final
    /// `0.2.0` as an update (same version, nothing newer); testers running
    /// a pre-release reinstall by hand.
    pub fn current() -> Version {
        Version::from_crate_version(env!("CARGO_PKG_VERSION"))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Where releases are looked up and downloaded from.
///
/// `github()` is the only source a shipped build ever uses; `loopback()`
/// exists for the manual end-to-end test and the integration tests, and
/// refuses anything that is not this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The `releases/latest` endpoint.
    pub api_latest: String,
    /// The prefix every accepted asset URL must start with.
    pub download_base: String,
}

impl Source {
    pub fn github() -> Source {
        Source {
            api_latest: format!("https://api.github.com/repos/{REPO}/releases/latest"),
            download_base: format!("https://github.com/{REPO}/releases/download/"),
        }
    }

    /// Accepts exactly `http://127.0.0.1:<port>/` or `http://localhost:<port>/`.
    pub fn loopback(url: &str) -> Result<Source, UpdateError> {
        let bad = || UpdateError::BadSource(url.to_string());
        let rest = url.strip_prefix("http://").ok_or_else(bad)?;
        let (host_port, path) = rest.split_once('/').ok_or_else(bad)?;
        let (host, port) = host_port.split_once(':').ok_or_else(bad)?;
        let local = host == "127.0.0.1" || host == "localhost";
        let numeric = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        if !local || !numeric || !path.is_empty() {
            return Err(bad());
        }
        Ok(Source {
            api_latest: format!("{url}releases/latest"),
            download_base: url.to_string(),
        })
    }
}

/// One published release, as much of it as the updater needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release body, Markdown as GitHub stores it.
    pub notes: String,
    /// The release page, for *View Release*.
    pub page_url: String,
    pub archive_url: String,
    pub sums_url: String,
    /// `Vitals-<version>-arm64.tar.gz`, the name looked up in `SHA256SUMS`.
    pub archive_name: String,
}

/// The tarball's name for a version, per the asset contract.
pub fn archive_name(version: Version) -> String {
    format!("Vitals-{version}-arm64.tar.gz")
}

/// Read GitHub's `releases/latest` response.
///
/// Rejects drafts and pre-releases, requires both assets by name, and
/// requires each asset's `browser_download_url` to start with
/// `source.download_base` and end with the asset's own name — a release
/// object is data from the network, and the only URLs it may send us to
/// are ones under our own prefix.
pub fn parse_latest(json: &[u8], source: &Source) -> Result<Release, UpdateError> {
    let v: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| UpdateError::BadRelease(format!("not JSON: {e}")))?;
    if v["draft"].as_bool() == Some(true) {
        return Err(UpdateError::BadRelease("latest release is a draft".into()));
    }
    if v["prerelease"].as_bool() == Some(true) {
        return Err(UpdateError::BadRelease(
            "latest release is a pre-release".into(),
        ));
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::BadRelease("no tag_name".into()))?;
    let version =
        Version::parse(tag).map_err(|e| UpdateError::BadRelease(format!("tag_name: {e}")))?;
    let assets = v["assets"]
        .as_array()
        .ok_or_else(|| UpdateError::BadRelease("no assets".into()))?;
    let name = archive_name(version);
    let archive_url = asset_url(assets, &name, source)?;
    let sums_url = asset_url(assets, SUMS_NAME, source)?;
    let page_url = v["html_url"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/v{version}"));
    Ok(Release {
        version,
        notes: v["body"].as_str().unwrap_or("").to_string(),
        page_url,
        archive_url,
        sums_url,
        archive_name: name,
    })
}

fn asset_url(
    assets: &[serde_json::Value],
    name: &str,
    source: &Source,
) -> Result<String, UpdateError> {
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(name))
        .ok_or_else(|| UpdateError::BadRelease(format!("release has no asset named {name}")))?;
    let url = asset["browser_download_url"].as_str().ok_or_else(|| {
        UpdateError::BadRelease(format!("asset {name} has no browser_download_url"))
    })?;
    let under_prefix = url.starts_with(&source.download_base);
    let named_right = url.rsplit('/').next() == Some(name);
    if !under_prefix || !named_right {
        return Err(UpdateError::BadRelease(format!(
            "asset {name} is served from {url}, not from under {}",
            source.download_base
        )));
    }
    Ok(url.to_string())
}

/// Every well-formed line of a `SHA256SUMS` file as `(hex digest, file name)`.
///
/// Accepts `shasum -a 256`'s two-space form and the `*name` binary marker
/// some tools emit; skips anything that is not 64 hex digits followed by
/// a name. Digests come back lower-case.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (hex, rest) = line.trim().split_once(char::is_whitespace)?;
            let name = rest.trim_start().trim_start_matches('*');
            let hex_ok = hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit());
            if !hex_ok || name.is_empty() {
                return None;
            }
            Some((hex.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

/// The digest `SHA256SUMS` records for `name`, as bytes.
pub fn expected_sum(sums: &str, name: &str) -> Option<[u8; 32]> {
    let (hex, _) = parse_sums(sums).into_iter().find(|(_, n)| n == name)?;
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Everything that can go wrong between "is there an update?" and "it is
/// installed". Each variant's string is the reason shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    BadVersion(String),
    BadSource(String),
    BadRelease(String),
    Http(String),
    BadChecksum,
    BadSignature(String),
    Io(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::BadVersion(s) => write!(f, "bad version {s}"),
            UpdateError::BadSource(s) => write!(f, "update source must be http://127.0.0.1:<port>/ or http://localhost:<port>/, not {s:?}"),
            UpdateError::BadRelease(s) => write!(f, "bad release: {s}"),
            UpdateError::Http(s) => write!(f, "{s}"),
            UpdateError::BadChecksum => write!(f, "the download's SHA-256 does not match SHA256SUMS"),
            UpdateError::BadSignature(s) => write!(f, "the update's code signature was rejected: {s}"),
            UpdateError::Io(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        UpdateError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's `releases/latest` shape, reduced to the fields the parser
    /// reads plus a decoy asset (the DMG) it must ignore.
    const LATEST: &str = r####"{
      "tag_name": "v0.2.0",
      "name": "Vitals 0.2.0",
      "draft": false,
      "prerelease": false,
      "html_url": "https://github.com/billsun9305/vitals/releases/tag/v0.2.0",
      "body": "### Added\n- Things.\n",
      "assets": [
        {"name": "Vitals-0.2.0.dmg",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0.dmg"},
        {"name": "Vitals-0.2.0-arm64.tar.gz",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz"},
        {"name": "SHA256SUMS",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/SHA256SUMS"}
      ]
    }"####;

    fn latest_with(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut v: serde_json::Value = serde_json::from_str(LATEST).unwrap();
        edit(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn version_parses_with_and_without_the_v() {
        let v = Version {
            major: 0,
            minor: 2,
            patch: 0,
        };
        assert_eq!(Version::parse("0.2.0"), Ok(v));
        assert_eq!(Version::parse("v0.2.0"), Ok(v));
        assert_eq!(Version::parse(" v0.2.0\n"), Ok(v));
        assert_eq!(v.to_string(), "0.2.0");
    }

    #[test]
    fn version_rejects_anything_but_three_numbers() {
        for bad in ["0.2.0-beta.1", "0.2", "0.2.0.1", "a.b.c", "", "v"] {
            assert!(
                matches!(Version::parse(bad), Err(UpdateError::BadVersion(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.1.1") > v("0.1.0"));
        assert_eq!(v("0.1.0"), v("v0.1.0"));
        assert_eq!(v("0.10.0").to_string(), "0.10.0");
    }

    #[test]
    fn current_version_is_the_crate_version() {
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn from_crate_version_never_panics_on_a_pre_release_suffix() {
        let v = |major, minor, patch| Version {
            major,
            minor,
            patch,
        };
        assert_eq!(Version::from_crate_version("0.2.0-beta.1"), v(0, 2, 0));
        assert_eq!(Version::from_crate_version("0.1.0"), v(0, 1, 0));
        assert_eq!(Version::from_crate_version("garbage"), v(0, 0, 0));
    }

    #[test]
    fn latest_json_parses_to_a_release() {
        let release = parse_latest(LATEST.as_bytes(), &Source::github()).unwrap();
        assert_eq!(release.version, Version::parse("0.2.0").unwrap());
        assert_eq!(release.notes, "### Added\n- Things.\n");
        assert_eq!(
            release.page_url,
            "https://github.com/billsun9305/vitals/releases/tag/v0.2.0"
        );
        assert_eq!(release.archive_name, "Vitals-0.2.0-arm64.tar.gz");
        assert_eq!(
            release.archive_url,
            "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz"
        );
        assert_eq!(
            release.sums_url,
            "https://github.com/billsun9305/vitals/releases/download/v0.2.0/SHA256SUMS"
        );
    }

    #[test]
    fn drafts_and_pre_releases_are_rejected() {
        let draft = latest_with(|v| v["draft"] = serde_json::Value::Bool(true));
        let pre = latest_with(|v| v["prerelease"] = serde_json::Value::Bool(true));
        for json in [draft, pre] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn a_suffixed_tag_is_a_bad_release_not_a_bad_version() {
        let json = latest_with(|v| v["tag_name"] = "v0.2.0-rc.1".into());
        assert!(matches!(
            parse_latest(&json, &Source::github()),
            Err(UpdateError::BadRelease(_))
        ));
    }

    #[test]
    fn a_release_missing_an_asset_is_rejected() {
        let no_tarball = latest_with(|v| {
            v["assets"]
                .as_array_mut()
                .unwrap()
                .retain(|a| a["name"] != "Vitals-0.2.0-arm64.tar.gz")
        });
        let no_sums = latest_with(|v| {
            v["assets"]
                .as_array_mut()
                .unwrap()
                .retain(|a| a["name"] != "SHA256SUMS")
        });
        let no_assets = latest_with(|v| v["assets"] = serde_json::Value::Null);
        for json in [no_tarball, no_sums, no_assets] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn an_asset_served_from_elsewhere_is_rejected() {
        // Another host with the right file name.
        let other_host = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://evil.example/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz".into()
        });
        // Our host, but a URL whose last segment is not the asset's name.
        let other_name = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://github.com/billsun9305/vitals/releases/download/v0.2.0/other.tar.gz".into()
        });
        // Our host, but not under releases/download/.
        let other_path = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://github.com/billsun9305/vitals/archive/Vitals-0.2.0-arm64.tar.gz".into()
        });
        for json in [other_host, other_name, other_path] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn not_json_is_a_bad_release() {
        assert!(matches!(
            parse_latest(b"<html>rate limited</html>", &Source::github()),
            Err(UpdateError::BadRelease(_))
        ));
    }

    #[test]
    fn sums_parse_and_look_up_by_name() {
        let sums = "\
0000000000000000000000000000000000000000000000000000000000000000  Vitals-0.2.0-arm64.tar.gz
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff *Vitals-0.2.0.dmg
this line is junk
abc  too-short.txt
";
        assert_eq!(
            parse_sums(sums),
            vec![
                ("0".repeat(64), "Vitals-0.2.0-arm64.tar.gz".to_string()),
                ("f".repeat(64), "Vitals-0.2.0.dmg".to_string()),
            ]
        );
        assert_eq!(
            expected_sum(sums, "Vitals-0.2.0-arm64.tar.gz"),
            Some([0u8; 32])
        );
        assert_eq!(expected_sum(sums, "Vitals-0.2.0.dmg"), Some([0xffu8; 32]));
        assert_eq!(expected_sum(sums, "missing"), None);
        // Upper-case hex is accepted.
        let upper = format!("{}  x\n", "AB".repeat(32));
        assert_eq!(expected_sum(&upper, "x"), Some([0xabu8; 32]));
    }

    #[test]
    fn github_source_points_at_the_repository() {
        let s = Source::github();
        assert_eq!(
            s.api_latest,
            "https://api.github.com/repos/billsun9305/vitals/releases/latest"
        );
        assert_eq!(
            s.download_base,
            "https://github.com/billsun9305/vitals/releases/download/"
        );
    }

    #[test]
    fn loopback_accepts_only_local_http_bases() {
        let s = Source::loopback("http://127.0.0.1:8000/").unwrap();
        assert_eq!(s.api_latest, "http://127.0.0.1:8000/releases/latest");
        assert_eq!(s.download_base, "http://127.0.0.1:8000/");
        assert!(Source::loopback("http://localhost:9/").is_ok());
        for bad in [
            "https://127.0.0.1:8000/",
            "http://127.0.0.1:8000",
            "http://127.0.0.1/",
            "http://127.0.0.2:8000/",
            "http://example.com:8000/",
            "http://127.0.0.1:8000/sub/",
            "http://127.0.0.1:80x/",
            "",
        ] {
            assert!(
                matches!(Source::loopback(bad), Err(UpdateError::BadSource(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn errors_display_their_reason() {
        assert_eq!(
            UpdateError::BadChecksum.to_string(),
            "the download's SHA-256 does not match SHA256SUMS"
        );
        assert_eq!(
            UpdateError::Http("x: HTTP 404".into()).to_string(),
            "x: HTTP 404"
        );
        let io: UpdateError = std::io::Error::other("disk full").into();
        assert_eq!(io.to_string(), "disk full");
    }
}
