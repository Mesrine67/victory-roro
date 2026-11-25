//! Hooks for the Tracy profiler (<https://github.com/wolfpld/tracy>), through
//! the `tracy-client` crate (<https://github.com/nagisa/rust_tracy_client>).
//!
//! Built only with the `tracy` feature
//! (`cargo build --profile diag -p victory --features diagnostics,tracy`);
//! without it every hook here is nothing.
//!
//! - [`zone!`](crate::zone): the rest of the enclosing block is a zone on
//!   this thread's timeline.
//! - [`frame`]: the title's present, which ends a frame.
//! - [`plot!`](crate::plot): a number drawn over time.
//!
//! The client collects nothing until a profiler connects (`ondemand`), and
//! listens on this machine only. The bundled client is Tracy 0.14.1: the
//! profiler that views it has to be 0.14.1 as well.

#[cfg(feature = "tracy")]
#[doc(hidden)]
pub use tracy_client as client;

/// The rest of the enclosing block as a named zone.
#[cfg(feature = "tracy")]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {
        let _zone = $crate::prof::client::span!($name);
    };
}

/// The rest of the enclosing block as a named zone (nothing in this build).
#[cfg(not(feature = "tracy"))]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {};
}

/// A value of the named plot.
#[cfg(feature = "tracy")]
#[macro_export]
macro_rules! plot {
    ($name:literal, $value:expr) => {
        $crate::prof::client::plot!($name, f64::from($value));
    };
}

/// A value of the named plot (nothing in this build).
#[cfg(not(feature = "tracy"))]
#[macro_export]
macro_rules! plot {
    ($name:literal, $value:expr) => {
        let _ = &$value;
    };
}

/// The title presented a frame.
#[inline]
pub fn frame() {
    #[cfg(feature = "tracy")]
    {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            tracing::info!(
                "Tracy hooks are in this build (client of Tracy 0.14.1, this machine only): \
                 connect the 0.14.1 profiler or tracy-capture to see zones, frames and samples"
            );
        });
        tracy_client::frame_mark();
    }
}
