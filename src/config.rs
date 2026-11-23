//! 設定読込。優先順位は Python 版と同じく `INI > 環境変数 > デフォルト`。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use ini::Ini;

pub const DEFAULT_TTL_SECS: u64 = 10;

/// `CLOUD_ENV` に指定できるクラウド。`tencent` は値としてのみ受理し、探索は未実装。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudEnv {
    Aws,
    Tencent,
}

impl CloudEnv {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "aws" => Some(Self::Aws),
            "tencent" => Some(Self::Tencent),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Tencent => "tencent",
        }
    }
}

/// クラウドごとの探索条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFilter {
    pub cloud_env: CloudEnv,
    pub tag_key: String,
    pub tag_value: String,
}

impl CloudFilter {
    /// タグ key/value とも未指定。
    pub fn tag_is_empty(&self) -> bool {
        self.tag_key.is_empty() && self.tag_value.is_empty()
    }

    /// タグ key/value の片方だけが指定されている。
    pub fn tag_is_partial(&self) -> bool {
        self.tag_key.is_empty() != self.tag_value.is_empty()
    }

    /// タグ key/value とも指定されている。
    pub fn tag_is_full(&self) -> bool {
        !self.tag_key.is_empty() && !self.tag_value.is_empty()
    }
}

/// 入出力パス。既定値は Python 版と同一（`base_hosts` は WORKDIR 相対だったものを絶対パス化）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub hosts: PathBuf,
    pub setting: PathBuf,
    pub hosts_list: PathBuf,
    pub rds_hosts_list: PathBuf,
    pub redis_hosts_list: PathBuf,
    pub base_hosts: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            hosts: PathBuf::from("/etc/hosts"),
            setting: PathBuf::from("/etc/create_hosts/setting.ini"),
            hosts_list: PathBuf::from("/etc/create_hosts/hosts_list"),
            rds_hosts_list: PathBuf::from("/etc/create_hosts/rds_hosts_list"),
            redis_hosts_list: PathBuf::from("/etc/create_hosts/redis_hosts_list"),
            base_hosts: PathBuf::from("/etc/create_hosts/base_hosts"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub ttl: Duration,
    pub get_rds: bool,
    pub get_redis: bool,
    pub rds: CloudFilter,
    pub redis: CloudFilter,
    pub paths: Paths,
}

/// 環境変数の参照関数。テストから差し替えられるようにトレイトオブジェクトで受ける。
pub type EnvLookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

impl Config {
    /// プロセス環境変数を使って読み込む。
    pub fn load(setting_path: &Path) -> Result<Self> {
        Self::load_with(setting_path, |name| std::env::var(name).ok())
    }

    /// 環境変数取得を差し替えて読み込む（テスト用）。
    pub fn load_with(setting_path: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Self> {
        // Python 版は `ConfigParser.read()` なので、ファイルが無くてもエラーにしない。
        let ini = if setting_path.is_file() {
            Ini::load_from_file(setting_path)?
        } else {
            Ini::new()
        };

        let get = |section: &str, key: &str, env_name: &str, default: &str| -> String {
            if let Some(v) = ini_lookup(&ini, section, key) {
                return v;
            }
            env(env_name).unwrap_or_else(|| default.to_string())
        };

        let ttl_raw = get("DEFAULT", "TTL", "CREATE_HOST_TTL", "10");
        let ttl_secs: u64 = ttl_raw
            .trim()
            .parse()
            .map_err(|_| anyhow!("TTL={ttl_raw} is invalid. Please set an integer (seconds)."))?;

        let get_rds = parse_bool(&get(
            "DEFAULT",
            "GET_RDS_ENDPOINT",
            "GET_RDS_ENDPOINT",
            "false",
        ));
        let get_redis = parse_bool(&get(
            "DEFAULT",
            "GET_REDIS_ENDPOINT",
            "GET_REDIS_ENDPOINT",
            "false",
        ));

        // Python 版は「有効なときだけ」CLOUD_ENV を検証する。無効時に不正値があっても既定に落とす。
        let rds_env_raw = get("RDS", "CLOUD_ENV", "RDS_CLOUD_ENV", "aws");
        let rds_cloud = match CloudEnv::parse(&rds_env_raw) {
            Some(v) => v,
            None if get_rds => {
                return Err(anyhow!(
                    "RDS_CLOUD_ENV={rds_env_raw} is invalid. Please set [aws, tencent]"
                ))
            }
            None => CloudEnv::Aws,
        };

        let redis_env_raw = get("REDIS", "CLOUD_ENV", "REDIS_CLOUD_ENV", "aws");
        let redis_cloud = match CloudEnv::parse(&redis_env_raw) {
            Some(v) => v,
            None if get_redis => {
                return Err(anyhow!(
                    "REDIS_CLOUD_ENV={redis_env_raw} is invalid. Please set [aws, tencent]"
                ))
            }
            None => CloudEnv::Aws,
        };

        Ok(Self {
            ttl: Duration::from_secs(ttl_secs),
            get_rds,
            get_redis,
            rds: CloudFilter {
                cloud_env: rds_cloud,
                // タグはフラグに関係なく常に読む（Python 版と同じ）
                tag_key: get("RDS", "SELECT_TAG_KEY", "RDS_SELECT_TAG_KEY", ""),
                tag_value: get("RDS", "SELECT_TAG_VALUE", "RDS_SELECT_TAG_VALUE", ""),
            },
            redis: CloudFilter {
                cloud_env: redis_cloud,
                tag_key: get("REDIS", "SELECT_TAG_KEY", "REDIS_SELECT_TAG_KEY", ""),
                tag_value: get("REDIS", "SELECT_TAG_VALUE", "REDIS_SELECT_TAG_VALUE", ""),
            },
            paths: Paths::default(),
        })
    }

    /// 起動時の設定サマリ（Python 版のログ出力に相当）。
    pub fn summary(&self) -> String {
        format!(
            "[TTL]: {}s, [IS_GET_RDS]: {}, [IS_GET_REDIS]: {}\n\
             [RDS_CLOUD_ENV]: {}, [RDS_SELECT_TAG_KEY]: {}, [RDS_SELECT_TAG_VALUE]: {}\n\
             [REDIS_CLOUD_ENV]: {}, [REDIS_SELECT_TAG_KEY]: {}, [REDIS_SELECT_TAG_VALUE]: {}",
            self.ttl.as_secs(),
            self.get_rds,
            self.get_redis,
            self.rds.cloud_env.as_str(),
            self.rds.tag_key,
            self.rds.tag_value,
            self.redis.cloud_env.as_str(),
            self.redis.tag_key,
            self.redis.tag_value,
        )
    }
}

/// Python 版は真偽値を「小文字化して "true" のみ真」で判定する。
fn parse_bool(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("true")
}

/// セクション名・キー名とも大文字小文字を無視して探す。
/// (Python の ConfigParser はキーのみ無視するが、実ファイルは大文字キーなので実害はない)
fn ini_lookup(ini: &Ini, section: &str, key: &str) -> Option<String> {
    for (name, props) in ini.iter() {
        if !name.unwrap_or("").eq_ignore_ascii_case(section) {
            continue;
        }
        for (k, v) in props.iter() {
            if k.eq_ignore_ascii_case(key) {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn paths_default_to_python_layout() {
        let p = Paths::default();
        assert_eq!(p.hosts, PathBuf::from("/etc/hosts"));
        assert_eq!(p.setting, PathBuf::from("/etc/create_hosts/setting.ini"));
        assert_eq!(p.base_hosts, PathBuf::from("/etc/create_hosts/base_hosts"));
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let cfg = Config::load_with(Path::new("/nonexistent/setting.ini"), no_env).unwrap();
        assert_eq!(cfg.ttl.as_secs(), DEFAULT_TTL_SECS);
        assert!(!cfg.get_rds);
        assert!(!cfg.get_redis);
        assert_eq!(cfg.rds.cloud_env, CloudEnv::Aws);
        assert!(cfg.rds.tag_is_empty());
    }

    #[test]
    fn bool_parsing_only_accepts_true() {
        assert!(parse_bool("true"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool(" True "));
        assert!(!parse_bool("yes"));
        assert!(!parse_bool("1"));
        assert!(!parse_bool(""));
    }
}
