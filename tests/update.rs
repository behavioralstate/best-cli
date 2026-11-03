//! `best update` and an unknown command's error, end to end: a copy of the binary in a temporary install directory, and
//! a directory standing for the newest release through the seam a debug build reads. No test reads a real release, so
//! without that seam there are none.
#![cfg(debug_assertions)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// What the binary says it is; the tests' releases are this version, and their installs say they are older.
const THIS: &str = env!("CARGO_PKG_VERSION");
const OLDER: &str = "0.0.1";

/// The release workflow's name for this machine; `None` where there is no release, and so nothing to update to.
fn target() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-musl")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("aarch64-unknown-linux-musl")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else {
        None
    }
}

fn archive_name(target: &str) -> String {
    format!("best-{target}.{}", if cfg!(windows) { "zip" } else { "tar.gz" })
}

/// An archive as the release workflow makes it: `best-<target>/best[.exe]` and the documents beside it.
fn archive(target: &str, binary: &[u8]) -> Vec<u8> {
    let folder = format!("best-{target}");
    if cfg!(windows) {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file(format!("{folder}/README.md"), stored).unwrap();
        zip.write_all(b"read me").unwrap();
        zip.start_file(format!("{folder}/best.exe"), stored).unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap().into_inner()
    } else {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
        let mut tar = tar::Builder::new(gz);
        for (name, bytes) in [("README.md", &b"read me"[..]), ("best", binary)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, format!("{folder}/{name}"), bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// An install directory holding a copy of the binary, marked so it can be told from the release's, and an empty
/// directory for the newest release.
struct Rig {
    install: TempDir,
    releases: TempDir,
    exe: PathBuf,
    before: Vec<u8>,
}

impl Rig {
    fn new() -> Self {
        let install = TempDir::new().unwrap();
        let releases = TempDir::new().unwrap();
        let exe = install.path().join(if cfg!(windows) { "best.exe" } else { "best" });
        let mut before = release_binary();
        before.extend_from_slice(b"the installed one");
        std::fs::write(&exe, &before).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            install,
            releases,
            exe,
            before,
        }
    }

    /// The newest release is this version, holding `binary` as a real archive.
    fn publish(&self, target: &str, binary: &[u8]) {
        let bytes = archive(target, binary);
        let name = archive_name(target);
        std::fs::write(self.releases.path().join(&name), &bytes).unwrap();
        self.publish_sums(target, &hex_sha256(&bytes));
    }

    fn publish_sums(&self, target: &str, sum: &str) {
        let name = archive_name(target);
        let versioned = name.replacen("best-", &format!("best-{THIS}-"), 1);
        std::fs::write(
            self.releases.path().join("SHA256SUMS"),
            format!("{sum}  {versioned}\n{sum}  {name}\n"),
        )
        .unwrap();
    }

    /// The installed copy, believing it is `running`. A file just written can be busy for a moment on Linux, when
    /// another test forks while it is open.
    fn run_as(&self, running: &str, args: &[&str]) -> Output {
        for _ in 0..20 {
            let output = Command::new(&self.exe)
                .args(args)
                .env("BEST_TEST_RELEASES", self.releases.path())
                .env("BEST_TEST_RUNNING_VERSION", running)
                .env_remove("BEST_CA_FILE")
                .output();
            match output {
                Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(std::time::Duration::from_millis(50)),
                other => return other.unwrap(),
            }
        }
        panic!("the installed copy stayed busy");
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_as(OLDER, args)
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.install.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn assert_as_it_was(&self) {
        assert!(bytes_of(&self.exe) == self.before, "the binary is untouched");
        assert_eq!(self.files().len(), 1, "nothing is left beside it: {:?}", self.files());
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn stderr(output: &Output) -> String {
    text(&output.stderr)
}

fn stdout(output: &Output) -> String {
    text(&output.stdout)
}

fn bytes_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The real binary with nothing added: what a release of this version holds, and what says this version.
fn release_binary() -> Vec<u8> {
    bytes_of(Path::new(env!("CARGO_BIN_EXE_best")))
}

#[test]
fn help_names_the_way_to_update() {
    let rig = Rig::new();
    for flag in ["--help", "-h"] {
        let output = rig.run(&[flag]);
        assert!(output.status.success());
        assert!(stdout(&output).contains("best update"), "{}", stdout(&output));
    }
}

#[test]
fn an_unknown_command_on_an_older_install_says_both_versions_and_how_to_update() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish_sums(target, "abc");
    let output = rig.run(&["credential-of", "work"]);
    assert_eq!(output.status.code(), Some(2), "what it always was");
    assert_eq!(stdout(&output), "", "a pipe sees nothing");
    let said = stderr(&output);
    assert!(said.contains("unrecognized subcommand 'credential-of'"), "{said}");
    assert!(
        said.contains(&format!(
            "no such command; this is best {OLDER}, the newest is {THIS}: `best update` installs it"
        )),
        "{said}"
    );
    rig.assert_as_it_was();
}

#[test]
fn an_unknown_command_when_the_newest_cannot_be_read_still_says_how_to_update() {
    let rig = Rig::new();
    let output = rig.run(&["credential-of", "work"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let said = stderr(&output);
    assert!(said.contains("unrecognized subcommand 'credential-of'"), "{said}");
    assert!(said.contains("the newest could not be read"), "{said}");
    assert!(said.contains("`best update`"), "{said}");
}

#[test]
fn an_unknown_command_on_the_newest_does_not_send_the_person_to_update() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish_sums(target, "abc");
    let output = rig.run_as(THIS, &["nosuch"]);
    assert_eq!(output.status.code(), Some(2));
    let said = stderr(&output);
    assert!(said.contains("`best update` would change nothing"), "{said}");
    assert!(said.contains("`best --help` lists the commands"), "{said}");
}

#[test]
fn an_unknown_command_with_json_is_one_json_error_on_stderr() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish_sums(target, "abc");
    let output = rig.run(&["nosuch", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let error: Value = serde_json::from_str(stderr(&output).trim()).expect("one JSON document");
    assert_eq!(error["exit"], 2);
    assert_eq!(error["error"]["details"]["running"], OLDER);
    assert_eq!(error["error"]["details"]["newest"], THIS);
    assert_eq!(error["error"]["details"]["update"], "best update");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("unrecognized subcommand 'nosuch'"), "{message}");
    assert!(message.contains("`best update`"), "{message}");
}

#[test]
fn another_command_line_error_reads_no_release_and_is_as_it_was() {
    let rig = Rig::new();
    let output = rig.run(&["--no-such-flag"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!stderr(&output).contains("best update"), "{}", stderr(&output));
}

#[test]
fn check_says_the_versions_and_changes_nothing() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish(target, b"never installed");
    let output = rig.run(&["update", "--check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output).trim(),
        format!("This is best {OLDER}; the newest is {THIS}. `best update` installs it.")
    );
    rig.assert_as_it_was();

    let output = rig.run(&["update", "--check", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["running"], OLDER);
    assert_eq!(answer["newest"], THIS);
    assert_eq!(answer["updateAvailable"], true);
    assert_eq!(answer["update"], "best update");
    rig.assert_as_it_was();

    let output = rig.run_as(THIS, &["update", "--check"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Nothing to update."), "{}", stdout(&output));
    rig.assert_as_it_was();
}

#[test]
fn check_that_cannot_read_the_newest_fails_as_unreachable() {
    let rig = Rig::new();
    let output = rig.run(&["update", "--check"]);
    assert_eq!(output.status.code(), Some(6), "{}", stderr(&output));
    assert!(stderr(&output).contains("SHA256SUMS"), "{}", stderr(&output));
    rig.assert_as_it_was();

    let output = rig.run(&["update", "--check", "--json"]);
    assert_eq!(output.status.code(), Some(6));
    assert_eq!(stdout(&output), "");
    let error: Value = serde_json::from_str(stderr(&output).trim()).unwrap();
    assert_eq!(error["exit"], 6);
}

#[test]
fn update_replaces_the_running_binary_and_a_later_run_clears_what_it_moved_aside() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    let release = release_binary();
    rig.publish(target, &release);

    let output = rig.run(&["update"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stderr(&output);
    assert!(said.contains(&format!("Updated best {OLDER} to {THIS}")), "{said}");
    assert_eq!(stdout(&output), "", "said to the person, on stderr");
    assert!(bytes_of(&rig.exe) == release, "the release's binary is in place");

    if cfg!(windows) {
        assert_eq!(rig.files().len(), 2, "the old one, moved aside: {:?}", rig.files());
        let aside = rig.files().into_iter().find(|f| f.ends_with(".old")).unwrap();
        assert!(
            bytes_of(&rig.install.path().join(aside)) == rig.before,
            "aside is the old binary"
        );
    }

    // The old process has ended, though Windows may hold the file a moment longer.
    let mut version = String::new();
    for _ in 0..50 {
        version = stdout(&rig.run(&["--version"]));
        if rig.files().len() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(version.trim(), format!("best {THIS}"));
    assert_eq!(rig.files().len(), 1, "only the binary is left: {:?}", rig.files());
}

#[test]
fn update_says_in_json_what_it_did() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish(target, &release_binary());
    let output = rig.run(&["update", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["running"], OLDER);
    assert_eq!(answer["newest"], THIS);
    assert_eq!(answer["updated"], true);
    let name = rig.exe.file_name().unwrap().to_str().unwrap();
    assert!(answer["path"].as_str().unwrap().ends_with(name), "{answer}");
}

#[test]
fn update_on_the_newest_says_so_and_changes_nothing() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    // The sums alone: an archive it tried to take would fail the run.
    rig.publish_sums(target, "abc");
    let output = rig.run_as(THIS, &["update"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(&format!("best {THIS} is the newest release; nothing changed.")),
        "{}",
        stderr(&output)
    );
    rig.assert_as_it_was();

    let output = rig.run_as(THIS, &["update", "--json"]);
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["updated"], false);
    assert!(answer["path"].is_string(), "{answer}");
    rig.assert_as_it_was();
}

#[test]
fn a_failed_download_leaves_the_install_as_it_was_and_fails_as_unreachable() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish_sums(target, "abc");
    let output = rig.run(&["update"]);
    assert_eq!(output.status.code(), Some(6), "{}", stderr(&output));
    assert!(stderr(&output).contains("nothing changed"), "{}", stderr(&output));
    rig.assert_as_it_was();

    // What it said on the way comes first on stderr, as a sign-in's does; the error is the last line.
    let output = rig.run(&["update", "--json"]);
    assert_eq!(output.status.code(), Some(6));
    let said = stderr(&output);
    let error: Value = serde_json::from_str(said.lines().last().unwrap()).unwrap();
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("nothing changed"), "{message}");
    rig.assert_as_it_was();
}

#[test]
fn an_archive_that_is_not_what_its_checksum_says_leaves_the_install_as_it_was() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish(target, &release_binary());
    rig.publish_sums(target, &hex_sha256(b"something else"));
    let output = rig.run(&["update"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("checksum"), "{}", stderr(&output));
    rig.assert_as_it_was();
}

#[test]
fn a_binary_in_the_archive_that_does_not_run_leaves_the_install_as_it_was() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    rig.publish(target, b"this is not a program");
    let output = rig.run(&["update"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("nothing changed"), "{}", stderr(&output));
    rig.assert_as_it_was();
}

#[test]
fn a_cargo_install_build_is_told_how_to_update_and_left_as_it_is() {
    let Some(target) = target() else { return };
    let root = TempDir::new().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(
        root.path().join(".crates.toml"),
        "[v1]\n\"best-cli 0.3.0 (git+https://github.com/behavioralstate/best-cli#abc)\" = [\"best\"]\n",
    )
    .unwrap();
    let mut rig = Rig::new();
    let exe = bin.join(rig.exe.file_name().unwrap());
    std::fs::copy(&rig.exe, &exe).unwrap();
    rig.exe = exe;
    rig.publish(target, &release_binary());

    let output = rig.run(&["update"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("cargo install --git"), "{}", stderr(&output));
    assert!(bytes_of(&rig.exe) == rig.before, "the binary is untouched");

    let output = rig.run(&["update", "--check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("cargo install --git"), "{}", stdout(&output));
    assert!(bytes_of(&rig.exe) == rig.before, "the binary is untouched");

    let output = rig.run(&["nosuch"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cargo install --git"), "{}", stderr(&output));
    assert!(!stderr(&output).contains("`best update`"), "{}", stderr(&output));
}

#[test]
fn tenant_is_refused_and_a_bad_ca_file_does_not_stop_an_update() {
    let Some(target) = target() else { return };
    let rig = Rig::new();
    let output = rig.run(&["--tenant", "ws", "update", "--check"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--tenant has no meaning"),
        "{}",
        stderr(&output)
    );

    rig.publish_sums(target, "abc");
    let output = rig.run(&["--ca-file", "no-such-file.pem", "update", "--check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}
