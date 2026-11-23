//! バイナリの CLI 挙動テスト（DNS に依存しない範囲のみ）。

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hosts-creator"));
    // 実行環境の環境変数（CREATE_HOST_TTL など）を混入させない
    cmd.env_clear();
    cmd
}

fn tempdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        let path = dir.path().join(name);
        std::fs::write(path, body).unwrap();
    }
    dir
}

#[test]
fn check_config_accepts_valid_setting() {
    let dir = tempdir_with(&[(
        "setting.ini",
        "[DEFAULT]\nTTL = 10\nGET_RDS_ENDPOINT = False\n",
    )]);
    let out = bin()
        .arg("--config")
        .arg(dir.path().join("setting.ini"))
        .arg("--check-config")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("configuration is valid"));
}

#[test]
fn check_config_rejects_invalid_ttl() {
    let dir = tempdir_with(&[("setting.ini", "[DEFAULT]\nTTL = ten\n")]);
    let out = bin()
        .arg("--config")
        .arg(dir.path().join("setting.ini"))
        .arg("--check-config")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("TTL=ten"));
}

#[test]
fn missing_hosts_list_exits_with_failure() {
    let dir = tempdir_with(&[
        ("setting.ini", "[DEFAULT]\nTTL = 10\n"),
        ("hosts", "127.0.0.1\tlocalhost\n"),
    ]);
    let p = |name: &str| dir.path().join(name);
    let out = bin()
        .arg("--config")
        .arg(p("setting.ini"))
        .arg("--hosts-path")
        .arg(p("hosts"))
        .arg("--hosts-list")
        .arg(p("missing_list"))
        .arg("--base-hosts")
        .arg(p("base_hosts"))
        .arg("--rds-hosts-list")
        .arg(p("rds_hosts_list"))
        .arg("--redis-hosts-list")
        .arg(p("redis_hosts_list"))
        .arg("--once")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing_list"), "stderr={stderr}");
}

#[test]
fn help_lists_new_flags() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in ["--once", "--dry-run", "--check-config", "--hosts-path"] {
        assert!(stdout.contains(flag), "missing {flag} in help");
    }
}

#[test]
fn default_paths_match_python_layout() {
    // 既定値の確認はヘルプ出力で担保（実際の /etc/hosts には触らない）
    let out = bin().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("/etc/create_hosts/setting.ini"), "{stdout}");
    assert!(stdout.contains("/etc/hosts"), "{stdout}");
    assert!(Path::new("/etc/create_hosts").is_absolute());
}
