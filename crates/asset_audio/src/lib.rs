mod createfx;
mod ent_channel;
mod map_script_sound;
mod sound_catalog;
mod sound_load;
mod sound_load_iw5;
mod sound_load_t5;
mod sound_wma_t5;
mod zone_sound;

pub use asset_core::*;
pub use asset_transport::*;
pub use createfx::*;
pub use ent_channel::*;
pub use map_script_sound::*;
pub use sound_catalog::*;
pub use sound_load::*;
pub use sound_load_iw5::*;
pub use sound_load_t5::*;
pub use sound_wma_t5::*;
pub use zone_sound::*;

pub mod asset_graph {
    pub use asset_core::*;
}
pub mod discover {
    pub use asset_transport::*;
}
pub mod zone {
    pub use asset_transport::*;
}

static KEEP_SOUND_SAMPLES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether loaded sounds keep their samples. A game that plays no sound (a
/// dedicated server, `--headless`) keeps every alias and what each loaded
/// sound is (format, rate, length), not the samples only playback reads.
pub fn keep_sound_samples(keep: bool) {
    KEEP_SOUND_SAMPLES.store(keep, std::sync::atomic::Ordering::Relaxed);
}

pub fn keeps_sound_samples() -> bool {
    KEEP_SOUND_SAMPLES.load(std::sync::atomic::Ordering::Relaxed)
}
