//! One tracing setup for every binary.
//!
//! Filter with `VICTORY_LOG` (falls back to `RUST_LOG`), e.g.
//! `VICTORY_LOG=victory_loader=trace,info`. Debug and trace output exist
//! only in builds with the `diagnostics` feature; a plain release build
//! logs info and above whatever the filter asks for.
//!
//! A build with the `tracy` feature also sends every line to the Tracy
//! profiler and starts its client (see `victory-gpu/src/prof.rs`).

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The most verbose level this build can log.
const CAP: LevelFilter = if cfg!(feature = "diagnostics") {
    LevelFilter::TRACE
} else {
    LevelFilter::INFO
};

pub fn init(default_level: &str) {
    let filter = std::env::var("VICTORY_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .and_then(|v| EnvFilter::try_new(v).ok())
        .unwrap_or_else(|| EnvFilter::new(default_level));
    let lines = tracing_subscriber::registry().with(CAP).with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(std::io::stderr),
    );
    // With the `tracy` feature the same lines also go to the Tracy profiler
    // (as its messages), and the client starts here, before any zone.
    #[cfg(feature = "tracy")]
    let _ = lines.with(tracing_tracy::TracyLayer::default()).try_init();
    #[cfg(not(feature = "tracy"))]
    let _ = lines.try_init();
}
