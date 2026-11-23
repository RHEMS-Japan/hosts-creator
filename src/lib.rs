//! hosts-creator (Rust rewrite)
//!
//! 旧 Python 実装（2023 年版）と同一の設定・ファイル仕様で `/etc/hosts` を更新し続ける
//! デーモン。差分は README の「旧 Python 版（2023）との違い」を参照。

pub mod config;
pub mod discovery;
pub mod hosts;
pub mod logging;
pub mod resolver;
pub mod runner;

pub use config::Config;
pub use runner::{run, RunDeps, RunOptions};
