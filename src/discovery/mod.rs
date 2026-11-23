//! RDS / ElastiCache のエンドポイント探索。
//!
//! Python 版のタグ判定は RDS と Redis で非対称なので、そのまま再現する:
//! - RDS  : タグ key/value 両方一致 → 採用 / 両方未指定 → 全件 / 片方のみ → **0件**
//! - Redis: key と value が両方指定 → タグ一致のみ / それ以外（片方のみ含む） → **全件**
//!
//! 片方のみ指定は設定ミスの可能性が高いため、起動時に警告を出す（挙動は変えない）。

use anyhow::Result;
use async_trait::async_trait;

use crate::config::CloudFilter;

pub mod aws;

pub use aws::AwsDiscovery;

#[async_trait]
pub trait EndpointDiscovery: Send + Sync {
    async fn rds_endpoints(&self, filter: &CloudFilter) -> Result<Vec<String>>;
    async fn redis_endpoints(&self, filter: &CloudFilter) -> Result<Vec<String>>;
}

/// RDS のタグ判定（Python 版 `get_aws_rds_endpoint_by_tag` と同等）。
pub fn rds_matches(tags: &[(String, String)], filter: &CloudFilter) -> bool {
    if filter.tag_is_full() {
        tags.iter()
            .any(|(k, v)| k == &filter.tag_key && v == &filter.tag_value)
    } else {
        filter.tag_is_empty()
    }
}

/// Redis でタグフィルタを適用するか（Python 版 `get_aws_redis_endpoint_by_tag` と同等）。
pub fn redis_should_filter(filter: &CloudFilter) -> bool {
    filter.tag_is_full()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CloudEnv;

    fn filter(key: &str, value: &str) -> CloudFilter {
        CloudFilter {
            cloud_env: CloudEnv::Aws,
            tag_key: key.to_string(),
            tag_value: value.to_string(),
        }
    }

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn t(env: &str) -> Vec<(String, String)> {
        tags(&[("Environment", env), ("Team", "platform")])
    }

    #[test]
    fn rds_no_tag_takes_all() {
        assert!(rds_matches(&t("prod"), &filter("", "")));
    }

    #[test]
    fn rds_full_tag_filters() {
        assert!(rds_matches(&t("prod"), &filter("Environment", "prod")));
        assert!(!rds_matches(&t("dev"), &filter("Environment", "prod")));
    }

    #[test]
    fn rds_partial_tag_matches_nothing() {
        // Python 版の暗黙仕様を維持
        assert!(!rds_matches(&t("prod"), &filter("Environment", "")));
        assert!(!rds_matches(&t("prod"), &filter("", "prod")));
    }

    #[test]
    fn redis_partial_tag_falls_back_to_all() {
        assert!(redis_should_filter(&filter("Environment", "prod")));
        assert!(!redis_should_filter(&filter("Environment", "")));
        assert!(!redis_should_filter(&filter("", "prod")));
        assert!(!redis_should_filter(&filter("", "")));
    }
}
