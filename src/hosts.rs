//! `/etc/hosts` の生成・検証・書き込み。
//!
//! 重要な制約:
//! - `/etc/hosts` はコンテナ内で bind mount されていることがある。`rename(2)` は
//!   mount point を跨げない（EXDEV）ため、必ず **open(O_TRUNC) + write** で上書きする。
//! - 検証は書き込み**前**に行う（Python 版は書いてから検証していた）。

use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// hosts_list 系ファイルを読み、1行ずつホスト名にする。
pub fn read_list(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_list(&content))
}

/// 空行・1文字以下・`#` 始まりの行を除く（Python 版と同じ）。
pub fn parse_list(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| line.len() > 1 && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// base_hosts の内容に解決結果を追記して hosts 全文を作る。
pub fn render(base: &str, entries: &[(IpAddr, String)]) -> String {
    let mut out = String::with_capacity(base.len() + entries.len() * 48);
    out.push_str(base);
    // base_hosts が改行で終わっていない場合に壊れた行を作らないための安全弁。
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (ip, name) in entries {
        out.push_str(&format!("{ip} {name}\n"));
    }
    out
}

/// hosts 全行の先頭トークンが IPv4/IPv6 であることを検証する。コメント・空行は無視。
pub fn validate(content: &str) -> Result<()> {
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        if first.parse::<IpAddr>().is_err() {
            bail!("line {}: {first} is not a valid IP address", idx + 1);
        }
    }
    Ok(())
}

/// 既存ファイルの内容を truncate して書き込む（rename しない）。
pub fn commit_in_place(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to open {} for writing", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parses_list_skipping_comments_and_short_lines() {
        let raw =
            "# comment\n\n  \nexample.local\n a\n127.0.0.1 db.internal\n   # indented comment\n";
        assert_eq!(
            parse_list(raw),
            vec!["example.local", "127.0.0.1 db.internal"]
        );
    }

    #[test]
    fn renders_base_then_entries() {
        let base = "127.0.0.1\tlocalhost\n";
        let out = render(base, &[(ip("10.0.0.5"), "db.internal".to_string())]);
        assert_eq!(out, "127.0.0.1\tlocalhost\n10.0.0.5 db.internal\n");
    }

    #[test]
    fn render_adds_missing_trailing_newline() {
        let out = render("127.0.0.1 localhost", &[(ip("10.0.0.5"), "db".to_string())]);
        assert_eq!(out, "127.0.0.1 localhost\n10.0.0.5 db\n");
    }

    #[test]
    fn validate_accepts_v4_and_v6() {
        validate("127.0.0.1 localhost\n::1 ip6-localhost\n# c\n\n10.0.0.1 a b\n").unwrap();
    }

    #[test]
    fn validate_rejects_non_ip() {
        let err = validate("127.0.0.1 localhost\nabc host\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn commit_truncates_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(&path, "x".repeat(600)).unwrap();
        commit_in_place(&path, b"127.0.0.1 localhost\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "127.0.0.1 localhost\n"
        );
    }

    #[test]
    fn commit_does_not_change_inode() {
        // bind mount 対策の確認: rename していれば inode が変わる
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(&path, "old\n").unwrap();
        let before = std::fs::metadata(&path).unwrap();
        commit_in_place(&path, b"new\n").unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.ino(), after.ino());
    }
}
