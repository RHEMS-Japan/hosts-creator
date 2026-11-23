//! hosts-creator (Rust rewrite)
//!
//! Python 版 (`py-latest` ブランチの `hosts-creator.py`) と同一の設定・ファイル仕様で
//! `/etc/hosts` を更新し続けるデーモン。差分は README の "Python 版との違い" を参照。

pub mod config;
pub mod discovery;
pub mod hosts;
pub mod logging;
pub mod resolver;
pub mod runner;

pub use config::Config;
pub use runner::{run, RunDeps, RunOptions};
