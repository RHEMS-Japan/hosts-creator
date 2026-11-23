//! 起動処理と更新ループ。

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures::stream::{self, StreamExt};
use tracing::{error, info, warn};

use crate::config::{CloudEnv, CloudFilter, Config, Paths};
use crate::discovery::EndpointDiscovery;
use crate::hosts;
use crate::resolver::HostResolver;

/// 同時に投げる DNS クエリ数。Python 版は逐次だが、名前解決は待ち時間が支配的なので並列化する。
const RESOLVE_CONCURRENCY: usize = 8;

pub struct RunDeps {
    pub resolver: Arc<dyn HostResolver>,
    /// `GET_RDS_ENDPOINT` / `GET_REDIS_ENDPOINT` が無効なら None。
    pub discovery: Option<Arc<dyn EndpointDiscovery>>,
}

#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// 1周だけ実行して終了する。
    pub once: bool,
    /// 解決結果を stdout に出し、hosts ファイルを書き換えない。
    pub dry_run: bool,
}

pub async fn run(cfg: &Config, deps: &RunDeps, opts: &RunOptions) -> Result<()> {
    ensure_base_hosts(&cfg.paths)?;

    if cfg.get_rds {
        prepare_endpoint_list(cfg, deps, Kind::Rds).await?;
    }
    if cfg.get_redis {
        prepare_endpoint_list(cfg, deps, Kind::Redis).await?;
    }

    loop {
        info!("===== start ====");
        let content = build_once(cfg, deps).await?;
        if opts.dry_run {
            print!("{content}");
        } else {
            hosts::commit_in_place(&cfg.paths.hosts, content.as_bytes())?;
            info!("Success to update hosts");
        }
        info!("===== end ====");

        if opts.once {
            return Ok(());
        }
        if wait_or_shutdown(cfg.ttl).await? {
            return Ok(());
        }
    }
}

/// `base_hosts` は初回だけ作る。k8s が書いた行を退避するための設計なので毎回は作り直さない。
fn ensure_base_hosts(paths: &Paths) -> Result<()> {
    if paths.base_hosts.is_file() {
        info!("{} is already exists.", paths.base_hosts.display());
        return Ok(());
    }
    info!(
        "copy {} to {}",
        paths.hosts.display(),
        paths.base_hosts.display()
    );
    std::fs::copy(&paths.hosts, &paths.base_hosts).with_context(|| {
        format!(
            "failed to copy {} to {}",
            paths.hosts.display(),
            paths.base_hosts.display()
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Rds,
    Redis,
}

impl Kind {
    fn label(&self) -> &'static str {
        match self {
            Self::Rds => "rds_hosts_list",
            Self::Redis => "redis_hosts_list",
        }
    }
}

/// 起動時に一度だけエンドポイントを列挙し、解決できたものだけをリストファイルに書く。
async fn prepare_endpoint_list(cfg: &Config, deps: &RunDeps, kind: Kind) -> Result<()> {
    let (filter, list_path) = match kind {
        Kind::Rds => (&cfg.rds, &cfg.paths.rds_hosts_list),
        Kind::Redis => (&cfg.redis, &cfg.paths.redis_hosts_list),
    };

    warn_if_partial_tag(filter, kind);

    if filter.cloud_env == CloudEnv::Tencent {
        // Python 版はこの組み合わせで rds_hosts_list を開けずに落ちていた。
        // ここでは空ファイルを用意して「探索なし」で継続する。
        warn!(
            "{}: CLOUD_ENV=tencent is not supported yet. skipping endpoint discovery",
            kind.label()
        );
        write_endpoint_list(list_path, &[])?;
        return Ok(());
    }

    let discovery = deps
        .discovery
        .as_ref()
        .with_context(|| format!("{}: discovery is not configured", kind.label()))?;

    let candidates = match kind {
        Kind::Rds => discovery.rds_endpoints(filter).await?,
        Kind::Redis => discovery.redis_endpoints(filter).await?,
    };

    let mut kept = Vec::new();
    for endpoint in candidates {
        match deps.resolver.resolve_a(&endpoint).await {
            Ok(ip) => {
                info!("{ip} {endpoint}");
                kept.push(endpoint);
            }
            Err(e) => error!("{endpoint} is not found ({e})"),
        }
    }
    info!("{}: {} endpoint(s)", kind.label(), kept.len());
    write_endpoint_list(list_path, &kept)
}

fn write_endpoint_list(path: &std::path::Path, endpoints: &[String]) -> Result<()> {
    let mut body = String::new();
    for endpoint in endpoints {
        body.push_str(endpoint);
        body.push('\n');
    }
    hosts::commit_in_place(path, body.as_bytes())
}

/// タグの片方だけ指定は設定ミスが多い。挙動は Python 版のまま（RDS=0件 / Redis=全件）。
fn warn_if_partial_tag(filter: &CloudFilter, kind: Kind) {
    if !filter.tag_is_partial() {
        return;
    }
    let consequence = match kind {
        Kind::Rds => "no instance will match",
        Kind::Redis => "all clusters will be selected",
    };
    warn!(
        "{}: only one of SELECT_TAG_KEY / SELECT_TAG_VALUE is set (key={:?}, value={:?}); {consequence}",
        kind.label(),
        filter.tag_key,
        filter.tag_value
    );
}

/// 1周分の hosts 全文を作る。DNS が1件でも失敗したら Err（部分反映しない）。
async fn build_once(cfg: &Config, deps: &RunDeps) -> Result<String> {
    let base = std::fs::read_to_string(&cfg.paths.base_hosts).with_context(|| {
        format!(
            "failed to read base hosts {}",
            cfg.paths.base_hosts.display()
        )
    })?;

    let mut names = hosts::read_list(&cfg.paths.hosts_list)?;
    if cfg.get_rds {
        names.extend(hosts::read_list(&cfg.paths.rds_hosts_list)?);
    }
    if cfg.get_redis {
        names.extend(hosts::read_list(&cfg.paths.redis_hosts_list)?);
    }

    let entries = resolve_all(deps.resolver.as_ref(), &names).await?;
    let content = hosts::render(&base, &entries);
    // Python 版は書き込んだ後に検証していた。ここでは書く前に検証する。
    hosts::validate(&content)?;
    Ok(content)
}

async fn resolve_all(
    resolver: &dyn HostResolver,
    names: &[String],
) -> Result<Vec<(IpAddr, String)>> {
    let futures = names.iter().enumerate().map(|(idx, name)| async move {
        let result = resolver.resolve_a(name).await;
        (idx, name, result)
    });
    let mut results: Vec<(usize, &String, Result<IpAddr>)> = stream::iter(futures)
        .buffer_unordered(RESOLVE_CONCURRENCY)
        .collect()
        .await;
    results.sort_by_key(|(idx, _, _)| *idx);

    let mut entries = Vec::with_capacity(results.len());
    for (_, name, result) in results {
        match result {
            Ok(ip) => {
                info!("{ip} {name}");
                entries.push((ip, name.clone()));
            }
            Err(e) => {
                error!("dns error");
                return Err(anyhow!("failed to resolve {name}: {e}"));
            }
        }
    }
    Ok(entries)
}

/// TTL 待機。SIGINT/SIGTERM を受けたら true（=正常終了）を返す。
async fn wait_or_shutdown(ttl: Duration) -> Result<bool> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::time::sleep(ttl) => Ok(false),
        _ = tokio::signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
            Ok(true)
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down");
            Ok(true)
        }
    }
}
