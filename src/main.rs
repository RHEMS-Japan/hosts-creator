use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::{error, info};

use hosts_creator::config::{Config, Paths};
use hosts_creator::discovery::{AwsDiscovery, EndpointDiscovery};
use hosts_creator::resolver::{HickoryResolver, HostResolver};
use hosts_creator::runner::{run, RunDeps, RunOptions};

/// 設定されたホスト名を解決して /etc/hosts を更新し続けるデーモン。
#[derive(Parser, Debug)]
#[command(name = "hosts-creator", version, about, long_about = None)]
struct Cli {
    /// setting.ini のパス
    #[arg(
        long,
        env = "SETTING_FILE",
        default_value = "/etc/create_hosts/setting.ini"
    )]
    config: PathBuf,

    /// 書き換え対象の hosts ファイル
    #[arg(long, env = "HOSTS_PATH", default_value = "/etc/hosts")]
    hosts_path: PathBuf,

    /// 解決対象のホスト名リスト
    #[arg(
        long,
        env = "HOSTS_LIST",
        default_value = "/etc/create_hosts/hosts_list"
    )]
    hosts_list: PathBuf,

    /// RDS エンドポイントの出力先
    #[arg(
        long,
        env = "RDS_HOSTS_LIST",
        default_value = "/etc/create_hosts/rds_hosts_list"
    )]
    rds_hosts_list: PathBuf,

    /// Redis エンドポイントの出力先
    #[arg(
        long,
        env = "REDIS_HOSTS_LIST",
        default_value = "/etc/create_hosts/redis_hosts_list"
    )]
    redis_hosts_list: PathBuf,

    /// 元になる hosts のバックアップ先（初回のみ作成）
    #[arg(
        long,
        env = "BASE_HOSTS_FILE",
        default_value = "/etc/create_hosts/base_hosts"
    )]
    base_hosts: PathBuf,

    /// 1周だけ実行して終了する
    #[arg(long)]
    once: bool,

    /// 解決結果を標準出力に出し、hosts ファイルは書き換えない
    #[arg(long)]
    dry_run: bool,

    /// 設定の検証だけ行う
    #[arg(long)]
    check_config: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    hosts_creator::logging::init();

    match real_main(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn real_main(cli: Cli) -> Result<()> {
    let mut cfg = Config::load(&cli.config)?;
    cfg.paths = Paths {
        hosts: cli.hosts_path.clone(),
        setting: cli.config.clone(),
        hosts_list: cli.hosts_list.clone(),
        rds_hosts_list: cli.rds_hosts_list.clone(),
        redis_hosts_list: cli.redis_hosts_list.clone(),
        base_hosts: cli.base_hosts.clone(),
    };

    info!("TTL={}", cfg.ttl.as_secs());
    info!("{}", cfg.summary());

    if cli.check_config {
        info!("configuration is valid");
        return Ok(());
    }

    let resolver: Arc<dyn HostResolver> = Arc::new(HickoryResolver::from_system_conf()?);
    let discovery: Option<Arc<dyn EndpointDiscovery>> = if cfg.get_rds || cfg.get_redis {
        Some(Arc::new(AwsDiscovery::new().await?))
    } else {
        None
    };

    let deps = RunDeps {
        resolver,
        discovery,
    };
    let opts = RunOptions {
        once: cli.once,
        dry_run: cli.dry_run,
    };
    run(&cfg, &deps, &opts).await
}
