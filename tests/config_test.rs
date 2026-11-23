//! 設定の優先順位・互換性テスト（Python 版 `LoadSettingIni` 相当）。

use std::path::Path;

use hosts_creator::config::{CloudEnv, Config};

fn no_env(_: &str) -> Option<String> {
    None
}

fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn ini_wins_over_env() {
    let cfg = Config::load_with(
        Path::new("testdata/setting_full.ini"),
        env_of(&[("CREATE_HOST_TTL", "99")]),
    )
    .unwrap();
    assert_eq!(cfg.ttl.as_secs(), 10);
    assert!(cfg.get_rds);
    assert!(cfg.get_redis);
    assert_eq!(cfg.rds.tag_key, "Environment");
    assert_eq!(cfg.redis.tag_value, "cache");
}

#[test]
fn env_used_when_ini_lacks_key() {
    let cfg = Config::load_with(
        Path::new("testdata/setting_min.ini"),
        env_of(&[("CREATE_HOST_TTL", "7"), ("GET_REDIS_ENDPOINT", "true")]),
    )
    .unwrap();
    assert_eq!(cfg.ttl.as_secs(), 7);
    assert!(!cfg.get_rds);
    assert!(cfg.get_redis);
    // タグはフラグが false でも読む（Python 版と同じ）
    assert_eq!(cfg.rds.tag_key, "Environment");
    assert_eq!(cfg.rds.tag_value, "staging");
}

#[test]
fn defaults_when_nothing_is_configured() {
    let cfg = Config::load_with(Path::new("testdata/does_not_exist.ini"), no_env).unwrap();
    assert_eq!(cfg.ttl.as_secs(), 10);
    assert!(!cfg.get_rds);
    assert!(!cfg.get_redis);
    assert_eq!(cfg.rds.cloud_env, CloudEnv::Aws);
    assert!(cfg.rds.tag_is_empty());
}

#[test]
fn only_lowercase_true_enables_flags() {
    let cfg = Config::load_with(
        Path::new("testdata/does_not_exist.ini"),
        env_of(&[("GET_RDS_ENDPOINT", "TRUE"), ("GET_REDIS_ENDPOINT", "1")]),
    )
    .unwrap();
    assert!(cfg.get_rds);
    assert!(!cfg.get_redis);
}

#[test]
fn ini_keys_are_case_insensitive() {
    let cfg = Config::load_with(Path::new("testdata/setting_lowercase.ini"), no_env).unwrap();
    assert_eq!(cfg.ttl.as_secs(), 7);
    assert!(cfg.get_rds);
}

#[test]
fn invalid_cloud_env_fails_when_enabled() {
    let err =
        Config::load_with(Path::new("testdata/setting_invalid_cloud.ini"), no_env).unwrap_err();
    assert!(err.to_string().contains("RDS_CLOUD_ENV=azure"), "{err}");
}

#[test]
fn invalid_cloud_env_is_ignored_when_disabled() {
    // GET_RDS_ENDPOINT が無いので CLOUD_ENV=azure は検証されず、既定 aws に落ちる
    let cfg = Config::load_with(Path::new("testdata/setting_cloud_disabled.ini"), no_env).unwrap();
    assert!(!cfg.get_rds);
    assert_eq!(cfg.rds.cloud_env, CloudEnv::Aws);
}

#[test]
fn invalid_ttl_is_rejected() {
    let err = Config::load_with(Path::new("testdata/setting_invalid_ttl.ini"), no_env).unwrap_err();
    assert!(err.to_string().contains("TTL=ten"), "{err}");
}

#[test]
fn cloud_env_parsing_accepts_tencent() {
    let cfg = Config::load_with(
        Path::new("testdata/does_not_exist.ini"),
        env_of(&[("GET_RDS_ENDPOINT", "true"), ("RDS_CLOUD_ENV", "Tencent")]),
    )
    .unwrap();
    assert_eq!(cfg.rds.cloud_env, CloudEnv::Tencent);
}

#[test]
fn summary_contains_python_field_names() {
    let cfg = Config::load_with(Path::new("testdata/setting_full.ini"), no_env).unwrap();
    let s = cfg.summary();
    assert!(s.contains("[TTL]: 10s"), "{s}");
    assert!(s.contains("[RDS_CLOUD_ENV]: aws"), "{s}");
    assert!(s.contains("[REDIS_SELECT_TAG_VALUE]: cache"), "{s}");
}
