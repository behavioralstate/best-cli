//! `best update`: the newest release over the running binary, found and checked as the install scripts do, from the
//! same repository and the same assets. `SHA256SUMS` under `releases/latest/download` names the newest version and the
//! checksum of the archive for this machine, so one small file answers both, with no API and no rate limit.
//!
//! Nothing is touched until the new binary is written beside the old one and has said its version, so a failed
//! download or a bad archive leaves the install as it was. No other command reads a release: nothing checks on every
//! use.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::http::{self, USER_AGENT};
use crate::output;
use crate::say;

const REPOSITORY: &str = "behavioralstate/best-cli";
const CHECKSUMS: &str = "SHA256SUMS";
/// The one way to update a build `best update` leaves alone.
const CARGO_UPDATE: &str = "cargo install --git https://github.com/behavioralstate/best-cli --force";
const UPDATE: &str = "best update";

/// The tests' seam, in debug builds only: a directory that stands for the newest release. A build people run takes
/// its releases from GitHub, whatever its environment says.
#[cfg(debug_assertions)]
const RELEASES_VAR: &str = "BEST_TEST_RELEASES";
/// The tests' seam, in debug builds only: the version this binary believes it is.
#[cfg(debug_assertions)]
const RUNNING_VAR: &str = "BEST_TEST_RUNNING_VERSION";

/// Where the newest release is read from: the seam the tests fake.
trait Releases {
    /// Where an asset is, as the person reads it.
    fn address(&self, name: &str) -> String;
    /// An asset of the newest release, whole.
    async fn asset(&self, name: &str) -> Result<Vec<u8>>;
}

/// The releases the install scripts take. Its client follows redirects, since `releases/latest/download` is one, and
/// is not `crate::http`'s: it carries no credential, trusts no `--ca-file` and speaks only https.
struct GitHub {
    client: reqwest::Client,
}

impl GitHub {
    fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120))
            .build()
            .context("building the HTTP client")?;
        Ok(Self { client })
    }
}

impl Releases for GitHub {
    fn address(&self, name: &str) -> String {
        format!("https://github.com/{REPOSITORY}/releases/latest/download/{name}")
    }

    async fn asset(&self, name: &str) -> Result<Vec<u8>> {
        let address = self.address(name);
        let broke = |e: reqwest::Error| http::Error::Unreachable(format!("{address}: {}", http::unreachable_text(&e)));
        let response = self.client.get(&address).send().await.map_err(broke)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(http::Error::Unreachable(format!("{address} answered {status}")).into());
        }
        Ok(response.bytes().await.map_err(broke)?.to_vec())
    }
}

#[cfg(debug_assertions)]
struct Directory(PathBuf);

#[cfg(debug_assertions)]
impl Releases for Directory {
    fn address(&self, name: &str) -> String {
        self.0.join(name).display().to_string()
    }

    async fn asset(&self, name: &str) -> Result<Vec<u8>> {
        let address = self.address(name);
        std::fs::read(&address).map_err(|e| http::Error::Unreachable(format!("{address}: {e}")).into())
    }
}

/// A release's version. A pre-release sorts before its release; build metadata does not count.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    numbers: [u64; 3],
    released: bool,
    text: String,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let bare = text.split('+').next().unwrap_or(text);
        let (core, released) = match bare.split_once('-') {
            Some((core, _)) => (core, false),
            None => (bare, true),
        };
        let numbers: Vec<u64> = core.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
        Some(Self {
            numbers: numbers.try_into().ok()?,
            released,
            text: text.to_owned(),
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// What this binary says it is.
fn running() -> Result<Version> {
    #[cfg(debug_assertions)]
    if let Ok(text) = std::env::var(RUNNING_VAR) {
        return Version::parse(&text).ok_or_else(|| anyhow!("{RUNNING_VAR} is not a version: {text}"));
    }
    Version::parse(env!("CARGO_PKG_VERSION")).context("this binary's version is not a version")
}

/// A machine a release is built for, named as the release workflow and the install scripts name it.
#[derive(Debug, Clone, Copy)]
struct Target {
    name: &'static str,
    windows: bool,
}

impl Target {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            windows: name.contains("windows"),
        }
    }

    fn this() -> Option<Self> {
        let name = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "x86_64-unknown-linux-musl"
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            "aarch64-unknown-linux-musl"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "aarch64-apple-darwin"
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            "x86_64-pc-windows-msvc"
        } else {
            return None;
        };
        Some(Self::new(name))
    }

    fn extension(self) -> &'static str {
        if self.windows {
            "zip"
        } else {
            "tar.gz"
        }
    }

    /// The archive's name without the version, which `releases/latest/download` resolves.
    fn archive(self) -> String {
        format!("best-{}.{}", self.name, self.extension())
    }

    /// Where the binary is inside the archive.
    fn binary(self) -> String {
        format!("best-{}/{}", self.name, if self.windows { "best.exe" } else { "best" })
    }
}

/// The newest release for a machine: its version and the checksum of its archive.
#[derive(Debug)]
struct Newest {
    version: Version,
    checksum: String,
}

/// `SHA256SUMS` holds each archive twice: under its versioned name, which says the version, and under the name without
/// it, which is the one downloaded.
fn newest_in(sums: &str, target: Target) -> Result<Newest> {
    let archive = target.archive();
    let versioned = format!("-{}.{}", target.name, target.extension());
    let (mut version, mut checksum) = (None, None);
    for line in sums.lines() {
        let mut words = line.split_whitespace();
        let (Some(sum), Some(name)) = (words.next(), words.next()) else {
            continue;
        };
        let name = name.trim_start_matches('*');
        if name == archive {
            checksum = Some(sum.to_ascii_lowercase());
        } else if let Some(text) = name.strip_prefix("best-").and_then(|n| n.strip_suffix(&versioned)) {
            version = Version::parse(text);
        }
    }
    match (version, checksum) {
        (Some(version), Some(checksum)) => Ok(Newest { version, checksum }),
        _ => bail!("the newest release names no archive for {}", target.name),
    }
}

async fn newest<R: Releases>(releases: &R, target: Target) -> Result<Newest> {
    let sums = releases.asset(CHECKSUMS).await?;
    let sums = String::from_utf8(sums).with_context(|| format!("{} is not text", releases.address(CHECKSUMS)))?;
    newest_in(&sums, target)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The binary out of an archive, read in memory: nothing else in it reaches the disk.
fn binary_in(archive: &[u8], target: Target) -> Result<Vec<u8>> {
    let wanted = target.binary();
    let mut binary = Vec::new();
    if target.windows {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))?;
        zip.by_name(&wanted)?.read_to_end(&mut binary)?;
        return Ok(binary);
    }
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(&wanted) {
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("it does not hold {wanted}")
}

/// The new binary beside the old one, on the same disk so a rename can put it in place. Gone again if it never gets
/// there.
struct Staged(PathBuf);

impl Staged {
    fn write(exe: &Path, binary: &[u8]) -> Result<Self> {
        let dir = exe.parent().context("this binary has no directory")?;
        let staged = Self(dir.join(format!(".best-update-{}.exe", uuid::Uuid::new_v4())));
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&staged.0)?;
            file.write_all(binary)?;
            make_executable(&file)?;
            file.sync_all()
        })();
        written.with_context(|| format!("writing the new binary in {}", dir.display()))?;
        Ok(staged)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
fn make_executable(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

/// The name a replaced binary takes while it may still be running: unique, because an older one may still be running
/// under the last such name and so cannot be removed yet.
fn aside_name(exe: &Path) -> PathBuf {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    exe.with_file_name(format!("{name}.{}.old", uuid::Uuid::new_v4()))
}

/// A running exe cannot be overwritten on Windows, but it can be renamed: the old one moves aside, the new one takes
/// its name, and when that fails the old one moves back.
fn replace_moving_aside(staged: &Path, exe: &Path) -> Result<PathBuf> {
    let aside = aside_name(exe);
    std::fs::rename(exe, &aside).with_context(|| format!("moving {} aside; nothing changed", exe.display()))?;
    if let Err(failed) = std::fs::rename(staged, exe) {
        return match std::fs::rename(&aside, exe) {
            Ok(()) => Err(anyhow!(failed).context(format!("replacing {}; nothing changed", exe.display()))),
            Err(_) => Err(anyhow!(failed).context(format!(
                "replacing {}: the binary is now {}, rename it back to use it",
                exe.display(),
                aside.display()
            ))),
        };
    }
    Ok(aside)
}

/// Removes what an update moved aside, on a later run, since a running exe cannot remove itself. One still running
/// stays for the run after that.
pub fn remove_old() {
    if cfg!(windows) {
        if let Ok(exe) = std::env::current_exe() {
            remove_old_beside(&exe);
        }
    }
}

fn remove_old_beside(exe: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if file.starts_with(&prefix) && file.ends_with(".old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What an installed binary is really called, a link followed where the system does not do it already.
fn this_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding this binary")?;
    if cfg!(windows) {
        return Ok(exe);
    }
    std::fs::canonicalize(&exe).with_context(|| format!("finding this binary from {}", exe.display()))
}

/// A build of cargo's and not an install script's: `cargo install` keeps its record in the root above `bin`, and
/// `cargo build` its fingerprints beside the binary. `best update` must not replace either behind cargo's back.
fn built_by_cargo(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    if dir.join(".fingerprint").is_dir() {
        return true;
    }
    dir.parent().is_some_and(|root| {
        [".crates2.json", ".crates.toml"]
            .iter()
            .any(|record| std::fs::read_to_string(root.join(record)).is_ok_and(|text| text.contains("best-cli ")))
    })
}

fn built_by_cargo_text() -> String {
    format!("this best was built by cargo, not installed from a release, so `best update` leaves it as it is. Update it with: {CARGO_UPDATE}")
}

fn no_release_text() -> String {
    format!("no release for this machine; build it with: cargo install --git https://github.com/{REPOSITORY}")
}

/// What became of `best update`.
#[derive(Debug)]
enum Outcome {
    /// Nothing newer: the install is as it was.
    Kept { newest: Version },
    /// In place; on Windows, the old one is where it was moved aside.
    Updated { newest: Version, aside: Option<PathBuf> },
}

/// `says` asks a binary for its `--version`.
async fn install<R: Releases>(
    releases: &R,
    exe: &Path,
    running: &Version,
    target: Target,
    says: impl Fn(&Path) -> Result<String>,
) -> Result<Outcome> {
    let Newest { version, checksum } = newest(releases, target).await?;
    if version <= *running {
        return Ok(Outcome::Kept { newest: version });
    }
    let name = target.archive();
    say!("Downloading {}", releases.address(&name));
    let archive = releases
        .asset(&name)
        .await
        .context("downloading the archive; nothing changed")?;
    let actual = sha256(&archive);
    if actual != checksum {
        bail!("the checksum of {name} does not match {CHECKSUMS}; nothing changed");
    }
    let binary = binary_in(&archive, target).with_context(|| format!("reading {name}; nothing changed"))?;
    let staged = Staged::write(exe, &binary).context("nothing changed")?;
    let expected = format!("best {version}");
    let said = says(&staged.0).with_context(|| format!("running the binary of {name}; nothing changed"))?;
    if said != expected {
        bail!("the binary of {name} says '{said}', not '{expected}'; nothing changed");
    }
    let aside = if target.windows {
        Some(replace_moving_aside(&staged.0, exe)?)
    } else {
        std::fs::rename(&staged.0, exe).with_context(|| format!("replacing {}; nothing changed", exe.display()))?;
        None
    };
    Ok(Outcome::Updated { newest: version, aside })
}

fn says_version(binary: &Path) -> Result<String> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        bail!("it exited with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `best update` and `best update --check`.
pub async fn run(check: bool, json: bool) -> Result<()> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os(RELEASES_VAR) {
        return update(&Directory(dir.into()), check, json).await;
    }
    update(&GitHub::new()?, check, json).await
}

async fn update<R: Releases>(releases: &R, check: bool, json: bool) -> Result<()> {
    let running = running()?;
    let exe = this_binary()?;
    let cargo = built_by_cargo(&exe);
    if cargo && !check {
        bail!(built_by_cargo_text());
    }
    let target = Target::this().with_context(no_release_text)?;
    if check {
        let newest = newest(releases, target).await?;
        return say_check(&running, &newest.version, cargo, json);
    }
    match install(releases, &exe, &running, target, says_version).await? {
        Outcome::Kept { newest } => {
            if json {
                return output::json(&json!({"running": running.text, "newest": newest.text, "updated": false}));
            }
            if newest == running {
                say!("best {running} is the newest release; nothing changed.");
            } else {
                say!("best {running} is ahead of the newest release, {newest}; nothing changed.");
            }
        }
        Outcome::Updated { newest, aside } => {
            if json {
                return output::json(
                    &json!({"running": running.text, "newest": newest.text, "updated": true, "path": exe}),
                );
            }
            say!("Updated best {running} to {newest}: {}", exe.display());
            if aside.is_some() {
                say!("The old binary was moved aside beside it; a later run of `best` removes it.");
            }
        }
    }
    Ok(())
}

/// The answer of `--check`, on stdout since it is what was asked for.
fn say_check(running: &Version, newest: &Version, cargo: bool, json: bool) -> Result<()> {
    let behind = newest > running;
    if json {
        let update = if cargo { CARGO_UPDATE } else { UPDATE };
        return output::json(&json!({
            "running": running.text,
            "newest": newest.text,
            "updateAvailable": behind,
            "update": update,
        }));
    }
    let how = match (behind, cargo) {
        (true, false) => format!("`{UPDATE}` installs it."),
        (true, true) => format!("This best was built by cargo; update it with: {CARGO_UPDATE}"),
        (false, _) => "Nothing to update.".to_owned(),
    };
    println!("This is best {running}; the newest is {newest}. {how}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    const LINUX: &str = "x86_64-unknown-linux-musl";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";

    /// A release in memory. An asset it does not hold fails as a download does.
    #[derive(Default)]
    struct Fake {
        assets: HashMap<String, Vec<u8>>,
        hangs: bool,
    }

    impl Releases for Fake {
        fn address(&self, name: &str) -> String {
            format!("fake/{name}")
        }

        async fn asset(&self, name: &str) -> Result<Vec<u8>> {
            if self.hangs {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            self.assets
                .get(name)
                .cloned()
                .ok_or_else(|| http::Error::Unreachable(format!("no {name}")).into())
        }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn tar_gz(target: Target, binary: &[u8]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(binary.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, target.binary(), binary).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn zipped(target: Target, binary: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(target.binary(), zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn archive_of(target: Target, binary: &[u8]) -> Vec<u8> {
        if target.windows {
            zipped(target, binary)
        } else {
            tar_gz(target, binary)
        }
    }

    fn sums(target: Target, version: &str, archive: &[u8]) -> Vec<u8> {
        let sum = sha256(archive);
        format!(
            "{sum}  best-{version}-{name}.{ext}\n{sum}  {alias}\n",
            name = target.name,
            ext = target.extension(),
            alias = target.archive()
        )
        .into_bytes()
    }

    fn release(target: Target, version: &str, archive: Vec<u8>) -> Fake {
        let mut fake = Fake::default();
        fake.assets.insert(CHECKSUMS.into(), sums(target, version, &archive));
        fake.assets.insert(target.archive(), archive);
        fake
    }

    /// An install directory holding only the old binary.
    fn installed() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("best");
        std::fs::write(&exe, b"the old binary").unwrap();
        (dir, exe)
    }

    fn files_in(dir: &TempDir) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn assert_as_it_was(dir: &TempDir, exe: &Path) {
        assert_eq!(std::fs::read(exe).unwrap(), b"the old binary");
        assert_eq!(files_in(dir), ["best"], "nothing is left beside it");
    }

    fn says_newest(_: &Path) -> Result<String> {
        Ok("best 0.3.0".to_owned())
    }

    #[test]
    fn versions_sort_as_numbers_and_a_pre_release_before_its_release() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert!(version("0.3.0") > version("0.3.0-dev"));
        assert_eq!(version("0.3.0+abc").numbers, [0, 3, 0]);
        assert!(Version::parse("0.3").is_none());
        assert!(Version::parse("0.3.0.1").is_none());
        assert!(Version::parse("latest").is_none());
    }

    #[test]
    fn the_sums_say_the_version_and_the_checksum_of_the_archive_for_this_machine() {
        // As the release workflow's `sha256sum best-*` writes them.
        let sums = "aaa  best-0.3.0-aarch64-apple-darwin.tar.gz\n\
                    BBB  best-0.3.0-x86_64-pc-windows-msvc.zip\n\
                    ccc  best-aarch64-apple-darwin.tar.gz\n\
                    BBB  best-x86_64-pc-windows-msvc.zip\n";
        let newest = newest_in(sums, Target::new(WINDOWS)).unwrap();
        assert_eq!(newest.version, version("0.3.0"));
        assert_eq!(newest.checksum, "bbb");
        let error = newest_in(sums, Target::new(LINUX)).unwrap_err().to_string();
        assert!(error.contains(LINUX), "{error}");
    }

    #[test]
    fn the_binary_is_read_from_either_kind_of_archive_by_the_path_the_workflow_gives_it() {
        for name in [LINUX, WINDOWS] {
            let target = Target::new(name);
            let archive = archive_of(target, b"the binary");
            assert_eq!(binary_in(&archive, target).unwrap(), b"the binary");
            assert!(binary_in(b"not an archive", target).is_err());
        }
        let linux = Target::new(LINUX);
        let elsewhere = archive_of(Target::new("aarch64-apple-darwin"), b"another");
        assert!(binary_in(&elsewhere, linux).is_err(), "not at the path it should be");
    }

    #[tokio::test]
    async fn a_newer_release_replaces_the_binary_in_place_and_leaves_nothing_else() {
        let target = Target::new(LINUX);
        let (dir, exe) = installed();
        let fake = release(target, "0.3.0", archive_of(target, b"the new binary"));
        let outcome = install(&fake, &exe, &version("0.1.2"), target, says_newest)
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Updated { aside: None, .. }), "{outcome:?}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"the new binary");
        assert_eq!(files_in(&dir), ["best"]);
    }

    #[tokio::test]
    async fn on_windows_the_old_binary_moves_aside_and_a_later_run_removes_it() {
        let target = Target::new(WINDOWS);
        let (dir, exe) = installed();
        let fake = release(target, "0.3.0", archive_of(target, b"the new binary"));
        let outcome = install(&fake, &exe, &version("0.1.2"), target, says_newest)
            .await
            .unwrap();
        let Outcome::Updated { aside: Some(aside), .. } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(std::fs::read(&exe).unwrap(), b"the new binary");
        assert_eq!(std::fs::read(&aside).unwrap(), b"the old binary");
        std::fs::write(dir.path().join("best.notes"), b"someone else's").unwrap();
        remove_old_beside(&exe);
        assert_eq!(files_in(&dir), ["best", "best.notes"]);
    }

    #[test]
    fn when_the_new_binary_cannot_take_the_name_the_old_one_moves_back() {
        let (dir, exe) = installed();
        let error = replace_moving_aside(&dir.path().join("not-there"), &exe).unwrap_err();
        assert!(format!("{error:#}").contains("nothing changed"), "{error:#}");
        assert_as_it_was(&dir, &exe);
    }

    #[tokio::test]
    async fn nothing_newer_changes_nothing_and_downloads_nothing() {
        let target = Target::new(LINUX);
        for running in ["0.3.0", "0.4.0"] {
            let (dir, exe) = installed();
            let mut fake = Fake::default();
            fake.assets
                .insert(CHECKSUMS.into(), sums(target, "0.3.0", b"never downloaded"));
            let outcome = install(&fake, &exe, &version(running), target, says_newest)
                .await
                .unwrap();
            assert!(matches!(outcome, Outcome::Kept { .. }), "{outcome:?}");
            assert_as_it_was(&dir, &exe);
        }
    }

    #[tokio::test]
    async fn a_failed_lookup_or_download_leaves_the_install_as_it_was() {
        let target = Target::new(LINUX);
        let (dir, exe) = installed();
        let running = version("0.1.2");
        let empty = Fake::default();
        assert!(install(&empty, &exe, &running, target, says_newest).await.is_err());
        assert_as_it_was(&dir, &exe);

        let mut sums_only = Fake::default();
        sums_only
            .assets
            .insert(CHECKSUMS.into(), sums(target, "0.3.0", b"an archive"));
        let error = install(&sums_only, &exe, &running, target, says_newest)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("nothing changed"), "{error:#}");
        assert!(
            error.chain().any(|c| c.is::<http::Error>()),
            "exits as unreachable: {error:#}"
        );
        assert_as_it_was(&dir, &exe);
    }

    #[tokio::test]
    async fn an_archive_that_does_not_match_its_checksum_is_not_opened() {
        let target = Target::new(LINUX);
        let (dir, exe) = installed();
        let mut fake = release(target, "0.3.0", archive_of(target, b"the new binary"));
        fake.assets.insert(target.archive(), b"something else".to_vec());
        let error = install(&fake, &exe, &version("0.1.2"), target, says_newest)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("checksum"), "{error:#}");
        assert_as_it_was(&dir, &exe);
    }

    #[tokio::test]
    async fn a_bad_archive_or_a_binary_that_says_otherwise_leaves_the_install_as_it_was() {
        let target = Target::new(LINUX);
        let (dir, exe) = installed();
        let running = version("0.1.2");

        let not_an_archive = release(target, "0.3.0", b"not an archive".to_vec());
        assert!(install(&not_an_archive, &exe, &running, target, says_newest)
            .await
            .is_err());
        assert_as_it_was(&dir, &exe);

        let good = release(target, "0.3.0", archive_of(target, b"the new binary"));
        let error = install(&good, &exe, &running, target, |_| Ok("best 0.2.0".to_owned()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("best 0.2.0"), "{error:#}");
        assert_as_it_was(&dir, &exe);

        let error = install(&good, &exe, &running, target, |_| bail!("it does not run"))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("it does not run"), "{error:#}");
        assert_as_it_was(&dir, &exe);
    }

    #[test]
    fn a_build_of_cargos_is_told_apart_by_what_cargo_keeps_around_it() {
        let root = TempDir::new().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let exe = bin.join("best");
        assert!(!built_by_cargo(&exe));
        std::fs::write(
            root.path().join(".crates.toml"),
            "[v1]\n\"other 1.0.0\" = [\"other\"]\n",
        )
        .unwrap();
        assert!(!built_by_cargo(&exe), "cargo installed something else here");
        std::fs::write(
            root.path().join(".crates.toml"),
            "[v1]\n\"best-cli 0.3.0 (git+https://github.com/behavioralstate/best-cli#abc)\" = [\"best\"]\n",
        )
        .unwrap();
        assert!(built_by_cargo(&exe));
        std::fs::remove_file(root.path().join(".crates.toml")).unwrap();
        std::fs::write(
            root.path().join(".crates2.json"),
            r#"{"installs":{"best-cli 0.3.0 (path+file:///x)":{}}}"#,
        )
        .unwrap();
        assert!(built_by_cargo(&exe));
        std::fs::remove_file(root.path().join(".crates2.json")).unwrap();
        std::fs::create_dir(bin.join(".fingerprint")).unwrap();
        assert!(built_by_cargo(&exe), "a `cargo build` target directory");
    }
}
