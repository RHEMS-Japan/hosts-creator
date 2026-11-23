//! AWS 実装（`aws-sdk-rds` / `aws-sdk-elasticache`）。
//!
//! 認証は AWS SDK の既定チェーン（IRSA / 環境変数 / IMDS）に任せる。
//! 必要な IAM 権限は README を参照。

use std::collections::HashSet;

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{rds_matches, redis_should_filter, EndpointDiscovery};
use crate::config::CloudFilter;

pub struct AwsDiscovery {
    rds: aws_sdk_rds::Client,
    elasticache: aws_sdk_elasticache::Client,
}

impl AwsDiscovery {
    pub async fn new() -> Result<Self> {
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        Ok(Self {
            rds: aws_sdk_rds::Client::new(&shared),
            elasticache: aws_sdk_elasticache::Client::new(&shared),
        })
    }
}

fn dedup_keep_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[async_trait]
impl EndpointDiscovery for AwsDiscovery {
    async fn rds_endpoints(&self, filter: &CloudFilter) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let mut pages = self.rds.describe_db_instances().into_paginator().send();
        while let Some(page) = pages.next().await {
            let page = page.context("describe_db_instances failed")?;
            for instance in page.db_instances() {
                let tags: Vec<(String, String)> = instance
                    .tag_list()
                    .iter()
                    .map(|tag| {
                        (
                            tag.key().unwrap_or_default().to_string(),
                            tag.value().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                if !rds_matches(&tags, filter) {
                    continue;
                }
                if let Some(address) = instance.endpoint().and_then(|e| e.address()) {
                    found.push(address.to_string());
                }
            }
        }
        Ok(dedup_keep_order(found))
    }

    async fn redis_endpoints(&self, filter: &CloudFilter) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let mut pages = self
            .elasticache
            .describe_cache_clusters()
            .show_cache_node_info(true)
            .into_paginator()
            .send();
        while let Some(page) = pages.next().await {
            let page = page.context("describe_cache_clusters failed")?;
            for cluster in page.cache_clusters() {
                if redis_should_filter(filter) {
                    // タグは ARN 単位でしか取れないため、クラスタごとに問い合わせる（Python 版同様）
                    let Some(arn) = cluster.arn() else {
                        continue;
                    };
                    let tags = self
                        .elasticache
                        .list_tags_for_resource()
                        .resource_name(arn)
                        .send()
                        .await
                        .with_context(|| format!("list_tags_for_resource failed: {arn}"))?;
                    let matched = tags.tag_list().iter().any(|tag| {
                        tag.key() == Some(filter.tag_key.as_str())
                            && tag.value() == Some(filter.tag_value.as_str())
                    });
                    if !matched {
                        continue;
                    }
                }
                for node in cluster.cache_nodes() {
                    if let Some(address) = node.endpoint().and_then(|e| e.address()) {
                        found.push(address.to_string());
                    }
                }
            }
        }
        Ok(dedup_keep_order(found))
    }
}
