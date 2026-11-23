//! ループ本体の挙動テスト（tempdir + StaticResolver）。

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use hosts_creator::config::{CloudEnv, CloudFilter, Config, Paths};
use hosts_creator::discovery::EndpointDiscovery;
use hosts_creator::resolver::{HostResolver, StaticResolver};
use hosts_creator::runner::{run, RunDeps, RunOptions};

struct StubDiscovery {
    rds: Vec<String>,
    redis: Vec<String>,
}

#[async_trait]
impl EndpointDiscovery for StubDiscovery {
    async fn rds_endpoints(&self, _filter: &CloudFilter) -> Result<Vec<String>> {
        Ok(self.rds.clone())
    }
    async fn redis_endpoints(&self, _filter: &CloudFilter) -> Result<Vec<String>> {
        Ok(self.redis.clone())
    }
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn paths(dir: &Path) -> Paths {
    Paths {
        hosts: dir.join("hosts"),
        setting: dir.join("setting.ini"),
        hosts_list: dir.join("hosts_list"),
        rds_hosts_list: dir.join("rds_hosts_list"),
        redis_hosts_list: dir.join("redis_hosts_list"),
        base_hosts: dir.join("base_hosts"),
    }
}

fn config(dir: &Path) -> Config {
    Config {
        ttl: Duration::from_millis(10),
        get_rds: false,
        get_redis: false,
        rds: CloudFilter {
            cloud_env: CloudEnv::Aws,
            tag_key: String::new(),
            tag_value: String::new(),
        },
        redis: CloudFilter {
            cloud_env: CloudEnv::Aws,
            tag_key: String::new(),
            tag_value: String::new(),
        },
        paths: paths(dir),
    }
}

/// tempdir に /etc/hosts 相当・hosts_list を用意する。
fn setup(base_hosts: &str, list: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    std::fs::write(&cfg.paths.hosts, "127.0.0.1\tlocalhost\n").unwrap();
    std::fs::write(&cfg.paths.hosts_list, list).unwrap();
    if !base_hosts.is_empty() {
        std::fs::write(&cfg.paths.base_hosts, base_hosts).unwrap();
    }
    (dir, cfg)
}

fn resolver() -> Arc<dyn HostResolver> {
    Arc::new(StaticResolver::new([
        ("db.internal", ip("10.0.0.5")),
        ("cache.internal", ip("10.0.0.6")),
    ]))
}

fn opts_once() -> RunOptions {
    RunOptions {
        once: true,
        dry_run: false,
    }
}

#[tokio::test]
async fn once_appends_resolved_entries_to_hosts() {
    let (_dir, cfg) = setup("", "db.internal\ncache.internal\n");
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    run(&cfg, &deps, &opts_once()).await.unwrap();

    let written = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    assert_eq!(
        written,
        "127.0.0.1\tlocalhost\n10.0.0.5 db.internal\n10.0.0.6 cache.internal\n"
    );
    // base_hosts が作られている
    assert!(cfg.paths.base_hosts.is_file());
}

#[tokio::test]
async fn dns_failure_leaves_hosts_untouched() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\nunknown.internal\n");
    let before = std::fs::read_to_string(&cfg.paths.hosts).unwrap();

    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    let err = run(&cfg, &deps, &opts_once()).await.unwrap_err();
    assert!(err.to_string().contains("unknown.internal"), "{err}");

    let after = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    assert_eq!(before, after, "失敗時は hosts を書き換えない");
}

#[tokio::test]
async fn base_hosts_is_not_refreshed_on_later_runs() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    run(&cfg, &deps, &opts_once()).await.unwrap();

    // 2周目の前に /etc/hosts が書き換わっても base_hosts は初回のまま
    std::fs::write(&cfg.paths.hosts, "10.9.9.9\tpods\n").unwrap();
    run(&cfg, &deps, &opts_once()).await.unwrap();

    let base = std::fs::read_to_string(&cfg.paths.base_hosts).unwrap();
    assert_eq!(base, "127.0.0.1\tlocalhost\n");
    let written = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    assert!(written.contains("10.0.0.5 db.internal"), "{written}");
    assert!(!written.contains("pods"), "{written}");
}

#[tokio::test]
async fn list_changes_are_picked_up_on_next_iteration() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    run(&cfg, &deps, &opts_once()).await.unwrap();
    assert!(!std::fs::read_to_string(&cfg.paths.hosts)
        .unwrap()
        .contains("cache.internal"));

    std::fs::write(&cfg.paths.hosts_list, "db.internal\ncache.internal\n").unwrap();
    run(&cfg, &deps, &opts_once()).await.unwrap();
    assert!(std::fs::read_to_string(&cfg.paths.hosts)
        .unwrap()
        .contains("10.0.0.6 cache.internal"));
}

#[tokio::test]
async fn dry_run_does_not_touch_hosts() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    let before = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    run(
        &cfg,
        &deps,
        &RunOptions {
            once: true,
            dry_run: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(before, std::fs::read_to_string(&cfg.paths.hosts).unwrap());
}

#[tokio::test]
async fn rds_discovered_endpoint_is_added_to_hosts() {
    let (_dir, mut cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    cfg.get_rds = true;

    let deps = RunDeps {
        resolver: resolver(),
        discovery: Some(Arc::new(StubDiscovery {
            rds: vec!["cache.internal".to_string()],
            redis: vec![],
        })),
    };
    run(&cfg, &deps, &opts_once()).await.unwrap();
    let written = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    assert!(written.contains("10.0.0.5 db.internal"), "{written}");
    assert!(written.contains("10.0.0.6 cache.internal"), "{written}");
}

#[tokio::test]
async fn enabled_discovery_without_backend_is_an_error() {
    let (_dir, mut cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    cfg.get_rds = true;
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    let err = run(&cfg, &deps, &opts_once()).await.unwrap_err();
    assert!(
        err.to_string().contains("discovery is not configured"),
        "{err}"
    );
}

#[tokio::test]
async fn rds_discovery_writes_only_resolvable_endpoints() {
    let (_dir, mut cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    cfg.get_rds = true;
    let deps = RunDeps {
        resolver: resolver(),
        discovery: Some(Arc::new(StubDiscovery {
            rds: vec![
                "found.rds.internal".to_string(),
                "cache.internal".to_string(),
            ],
            redis: vec![],
        })),
    };
    // found.rds.internal は StaticResolver に無いので落ちる -> 一覧から外れる
    run(&cfg, &deps, &opts_once()).await.unwrap();

    let list = std::fs::read_to_string(&cfg.paths.rds_hosts_list).unwrap();
    assert_eq!(list, "cache.internal\n");
    let written = std::fs::read_to_string(&cfg.paths.hosts).unwrap();
    assert!(written.contains("10.0.0.6 cache.internal"), "{written}");
}

#[tokio::test]
async fn tencent_skips_discovery_and_does_not_crash() {
    let (_dir, mut cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    cfg.get_rds = true;
    cfg.rds.cloud_env = CloudEnv::Tencent;
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    run(&cfg, &deps, &opts_once()).await.unwrap();
    // 空のリストが用意され、ループは通常どおり動く
    assert_eq!(
        std::fs::read_to_string(&cfg.paths.rds_hosts_list).unwrap(),
        ""
    );
    assert!(std::fs::read_to_string(&cfg.paths.hosts)
        .unwrap()
        .contains("10.0.0.5 db.internal"));
}

#[tokio::test]
async fn missing_hosts_list_is_an_error() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "");
    std::fs::remove_file(&cfg.paths.hosts_list).unwrap();
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    let err = run(&cfg, &deps, &opts_once()).await.unwrap_err();
    assert!(err.to_string().contains("hosts_list"), "{err}");
}

#[tokio::test]
async fn hosts_path_must_be_writable() {
    let (_dir, cfg) = setup("127.0.0.1\tlocalhost\n", "db.internal\n");
    let deps = RunDeps {
        resolver: resolver(),
        discovery: None,
    };
    // hosts ファイルを消して親ディレクトリごと無効化 -> 書き込み失敗
    let broken = PathBuf::from("/proc/self/nonexistent/hosts");
    let mut cfg2 = cfg.clone();
    cfg2.paths.hosts = broken;
    let err = run(&cfg2, &deps, &opts_once()).await.unwrap_err();
    assert!(!err.to_string().is_empty());
}
