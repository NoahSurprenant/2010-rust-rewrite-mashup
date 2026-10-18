use bevy::prelude::*;

/// Local skating presentation. Physics remains owned by the optional Skate host.
#[derive(Resource, Default)]
pub struct SkateMode {
    pub active: bool,
    pub entering: bool,
    pub preloaded: bool,
    pub preload_pending: bool,
    pub controller: Option<usize>,
    pub toggle_requested: bool,
    pub input_blocked: bool,
    pub client: u32,
    pub root: Mat4,
    pub bones: Vec<Mat4>,
    pub names: Vec<String>,
    pub camera: Option<(Transform, f32)>,
    pub tick: u64,
    pub status: String,
    /// A controller the console holds (`skate pad`), in place of a real one:
    /// for tests and machines without a controller.
    pub virtual_pad: Option<VirtualPad>,
    /// Map units a second, as the skate simulation last had it.
    pub speed: f32,
}

/// A controller's state in XInput's layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualPad {
    pub buttons: u16,
    pub triggers: [u8; 2],
    pub left: [i16; 2],
    pub right: [i16; 2],
}

/// A remote player on its board, as the host passed it on.
#[derive(Clone, Debug, Default)]
pub struct SkaterPose {
    /// The skater's root in the map.
    pub root: Mat4,
    /// Skate 3 bones relative to the root, named by `RemoteSkaters::names`.
    pub bones: Vec<Mat4>,
    /// The skate simulation's state, e.g. `PhysicsGround`.
    pub label: String,
    /// States received for this skater so far.
    pub received: u64,
}

/// Every other player on a board, by client: drawn as its skater and board
/// in place of its MW2 body.
#[derive(Resource, Default)]
pub struct RemoteSkaters {
    pub poses: std::collections::HashMap<u32, SkaterPose>,
    /// The bone names every pose's bones follow.
    pub names: Vec<String>,
    /// Times each skater was drawn on its board, by client (the renderer
    /// counts, through a shared reference).
    pub drawn: std::sync::Mutex<std::collections::BTreeMap<u32, u64>>,
}

impl RemoteSkaters {
    pub fn note_drawn(&self, client: u32) {
        if let Ok(mut drawn) = self.drawn.lock() {
            *drawn.entry(client).or_default() += 1;
        }
    }

    pub fn drawn(&self, client: u32) -> u64 {
        self.drawn
            .lock()
            .map(|d| d.get(&client).copied().unwrap_or(0))
            .unwrap_or(0)
    }
}
