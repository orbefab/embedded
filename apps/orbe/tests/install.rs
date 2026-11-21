//! `scripts/install.sh` bounds its downloads and says how to run the binary it installed.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn downloads_time_out_and_retry() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.sh");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(
        "curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 --speed-limit 1 --speed-time 30 -o /dev/null -w '%{url_effective}'"
    ));
    assert!(text.contains(
        "curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 --speed-limit 1 --speed-time 30 -o \"$dest\" \"$url\""
    ));
    assert!(text.contains("wget -q --timeout=15 --tries=3 -O \"$dest\" \"$url\""));
    let last = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap();
    assert_eq!(last, "main \"$@\"");
}

/// Off PATH, the next command is the installed binary. On PATH, it is `orbe --version`.
#[cfg(unix)]
#[test]
fn the_next_command_names_the_binary_when_its_folder_is_not_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("stage");
    fs::create_dir(&staged).unwrap();
    let binary = staged.join("orbe");
    fs::write(&binary, "#!/bin/sh\necho 'orbe 0.0.0'\n").unwrap();
    let mut mode = fs::metadata(&binary).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(&binary, mode).unwrap();
    let archive = root.path().join("orbe.tar.gz");
    let packed = Command::new("tar")
        .args([
            "-czf",
            archive.to_str().unwrap(),
            "-C",
            staged.to_str().unwrap(),
            "orbe",
        ])
        .status()
        .unwrap();
    assert!(packed.success());
    let sum = Command::new("sha256sum").arg(&archive).output().unwrap();
    assert!(sum.status.success());
    let hex = String::from_utf8(sum.stdout).unwrap();
    let hex = hex.split_whitespace().next().unwrap();
    fs::write(root.path().join("orbe.tar.gz.sha256"), format!("{hex}\n")).unwrap();

    let dest = root.path().join("dest");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.sh");
    let install = |path: &str| {
        let output = Command::new("sh")
            .arg(&script)
            .args([
                "--tarball",
                archive.to_str().unwrap(),
                "--dir",
                dest.to_str().unwrap(),
                "--version",
                "0.0.0",
            ])
            .env("PATH", path)
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let (ok, stdout, stderr) = install("/usr/bin:/bin");
    assert!(ok, "{stdout}{stderr}");
    let abs = dest.canonicalize().unwrap();
    let quoted = format!("\"{}/orbe\" --version", abs.display());
    assert!(stdout.lines().any(|line| line == quoted), "{stdout}");
    assert!(
        !stdout.lines().any(|line| line == "orbe --version"),
        "{stdout}"
    );
    assert!(stdout.contains("is not on PATH"), "{stdout}");

    let (ok, stdout, stderr) = install(&format!("{}:/usr/bin:/bin", abs.display()));
    assert!(ok, "{stdout}{stderr}");
    assert!(
        stdout.lines().any(|line| line == "orbe --version"),
        "{stdout}"
    );
    assert!(!stdout.lines().any(|line| line == quoted), "{stdout}");
    assert!(!stdout.contains("is not on PATH"), "{stdout}");
}
