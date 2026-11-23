//! Python 版のログ書式に近づけた tracing 設定。
//! 既存のログ検索（`[INFO ...]` 前提）を壊さないための互換レイヤ。
//!
//! Python: `[%(levelname)s %(asctime)s L%(lineno)d:%(name)s] message`
//! Rust:   `[LEVEL 2026-11-22 00:30:00 hosts_creator::runner] message`
//! （行番号は tracing のイベントから取得できないため target のみ）

use std::fmt;

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::{format::Writer, FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

struct PyStyleFormat;

impl<S, N> FormatEvent<S, N> for PyStyleFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        write!(
            writer,
            "[{} {} {}] ",
            meta.level(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            meta.target()
        )?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// `RUST_LOG` があればそれを使い、無ければ info。
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .event_format(PyStyleFormat)
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
