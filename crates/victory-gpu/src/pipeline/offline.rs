//! Host pipelines made before a run wants them (`victory precompile`):
//! each pipeline an earlier run listed (`manifest`) is put together again
//! from the title's own shader containers and its recorded state, and made
//! as a run makes it, so that it lands in the pipeline library under the
//! name the run looks it up by.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::manifest::{self, Created};
use super::{library, make};

/// The pipelines earlier runs listed, in the order the title first created
/// them, and for each whether a run has drawn with it (the early list).
/// Empty until a run with a pipeline cache has created some.
pub fn created_pipelines() -> Vec<(Created, bool)> {
    let Some(dir) = library::dir() else {
        return Vec::new();
    };
    manifest::created(dir)
        .into_iter()
        .map(|p| {
            let drawn = library::wanted_early(&p.key);
            (p, drawn)
        })
        .collect()
}

/// What became of a listed pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precompiled {
    /// It has a host pipeline, in the library now.
    Ready,
    /// A run does not make it either (and why).
    LeftOut(&'static str),
    /// One of its shaders was not found in the title's files.
    NoContainer,
    /// One of its shaders does not translate.
    Untranslated,
    /// The host refused it, or it has stages or a description that get no
    /// host pipeline.
    Refused,
    /// Its recorded state is not readable.
    Damaged,
}

/// Replayed pipelines whose key is not the one the run listed them under.
static OTHER_KEY: AtomicU64 = AtomicU64::new(0);

/// Makes the host pipeline of `p` on this thread, as a run would on its
/// first draw. `container` gives a shader's container by the first 8 bytes
/// of its BLAKE3.
pub fn precompile(p: &Created, container: &dyn Fn(&[u8; 8]) -> Option<Arc<[u8]>>) -> Precompiled {
    let mut shaders = Vec::with_capacity(p.shaders.len());
    for (stage, hash) in &p.shaders {
        let Some(bytes) = container(hash) else {
            return Precompiled::NoContainer;
        };
        shaders.push(super::shader(stage, &bytes));
    }
    let graphics = match &p.state {
        Some(state) => match manifest::replay(state) {
            Some(g) => Some(Box::new(g)),
            None => return Precompiled::Damaged,
        },
        None => None,
    };
    let pending = Arc::new(make::Pending::new(shaders, graphics));
    if pending.key() != p.key && OTHER_KEY.fetch_add(1, Ordering::Relaxed) == 0 {
        // Still made: its name in the library comes from what it is made of.
        tracing::warn!(
            listed = %p.key,
            replayed = %pending.key(),
            "a listed pipeline replays under another key (a list of another build?); logged once, counted"
        );
    }
    if let Some(why) = pending.left_out() {
        return Precompiled::LeftOut(why);
    }
    if make::now(&pending) {
        Precompiled::Ready
    } else if pending.untranslated() {
        Precompiled::Untranslated
    } else {
        Precompiled::Refused
    }
}

/// Host pipelines compiled and loaded from the library since the start,
/// replayed pipelines that had another key than listed, and the library's
/// size in bytes as it would be written now.
pub fn pipeline_counts() -> [u64; 4] {
    [
        library::MADE.load(Ordering::Relaxed),
        library::LOADED.load(Ordering::Relaxed),
        OTHER_KEY.load(Ordering::Relaxed),
        library::size(),
    ]
}
