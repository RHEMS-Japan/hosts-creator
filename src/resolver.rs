//! 名前解決の抽象化。DNS は hickory、テストは StaticResolver を使う。
//!
//! Python 版は dnspython の `Resolver(configure=True)` を使う。dnspython は
//! `/etc/hosts` を参照しないため、こちらも「DNS のみ」で解決する（glibc の
//! getaddrinfo を使うと hosts を参照してしまい挙動が変わる）。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

#[async_trait]
pub trait HostResolver: Send + Sync {
    /// A レコードの先頭を返す（Python 版の `resolver.resolve(rec, 'A')[0]` 相当）。
    async fn resolve_a(&self, name: &str) -> Result<IpAddr>;
}

/// 本番用: システムの `/etc/resolv.conf` を読む hickory リゾルバ。
pub struct HickoryResolver {
    inner: hickory_resolver::TokioAsyncResolver,
}

impl HickoryResolver {
    pub fn from_system_conf() -> Result<Self> {
        let (config, mut opts) = hickory_resolver::system_conf::read_system_conf()
            .map_err(|e| anyhow!("failed to read system resolver config: {e}"))?;
        // dnspython はキャッシュしないので鮮度を合わせる。
        // (フェイルオーバー直後の IP 変化を取りこぼさないため)
        opts.cache_size = 0;
        opts.attempts = 3;
        opts.timeout = Duration::from_secs(5);
        Ok(Self {
            inner: hickory_resolver::TokioAsyncResolver::tokio(config, opts),
        })
    }
}

#[async_trait]
impl HostResolver for HickoryResolver {
    async fn resolve_a(&self, name: &str) -> Result<IpAddr> {
        let lookup = self.inner.ipv4_lookup(name).await?;
        lookup
            .iter()
            .next()
            .map(|record| IpAddr::V4(record.0))
            .ok_or_else(|| anyhow!("no A record for {name}"))
    }
}

/// テスト用: 固定の対応表で解決する。
#[derive(Default)]
pub struct StaticResolver {
    map: HashMap<String, IpAddr>,
    fail_on: Vec<String>,
}

impl StaticResolver {
    pub fn new<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = (S, IpAddr)>,
        S: Into<String>,
    {
        Self {
            map: entries.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            fail_on: Vec::new(),
        }
    }

    pub fn fail_for(mut self, names: &[&str]) -> Self {
        self.fail_on = names.iter().map(|s| s.to_string()).collect();
        self
    }
}

#[async_trait]
impl HostResolver for StaticResolver {
    async fn resolve_a(&self, name: &str) -> Result<IpAddr> {
        if self.fail_on.iter().any(|n| n == name) {
            return Err(anyhow!("dns error (static): {name}"));
        }
        self.map
            .get(name)
            .copied()
            .ok_or_else(|| anyhow!("dns error (static): {name} not found"))
    }
}

pub type SharedResolver = Arc<dyn HostResolver>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_resolver_hits_and_misses() {
        let resolver =
            StaticResolver::new([("db.internal", "10.0.0.5".parse::<IpAddr>().unwrap())])
                .fail_for(&["boom.internal"]);
        assert_eq!(
            resolver.resolve_a("db.internal").await.unwrap(),
            "10.0.0.5".parse::<IpAddr>().unwrap()
        );
        assert!(resolver.resolve_a("missing.internal").await.is_err());
        assert!(resolver.resolve_a("boom.internal").await.is_err());
    }
}
