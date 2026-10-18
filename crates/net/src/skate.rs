//! Skating in multiplayer.
//!
//! A skater's own game runs the Skate 3 simulation (it needs the Skate 3
//! files, the controller and the map's collision, all of which it has) and
//! sends what came of it: where the skater is, how fast it goes, and its
//! pose. The host checks that loosely (alive, started where the player
//! stands, no faster than a skater can go) and moves that client's player in
//! its simulation there, so MW2's hits, collisions and kill credit see the
//! skater where it is. The host passes each skater's state on to everyone
//! else, who draw the skater's pose and board in place of the MW2 body. A
//! dedicated server only checks and relays: it never runs the skate
//! simulation and needs no Skate 3 files.
//!
//! `SkaterState` is the wire form, one per skater per update; the client
//! sends its own (`ClientPacket::Skate`, unreliable), the host sends the
//! others' in a trailing section of each snapshot.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::Resource;

use crate::transport::wire::{WireError, WireReader, WireWriter};

/// The Skate 3 bones a skater's pose carries, in wire order: the skater's
/// body and its board, all of the reference rig (`rig.json`).
pub const BONES: [&str; 35] = [
    "HIPS",
    "SPINE",
    "SPINE1",
    "SPINE2",
    "SPINE3",
    "NECK",
    "NECK1",
    "HEAD",
    "RIGHTSHOULDER",
    "RIGHTARM",
    "RIGHTFOREARM",
    "RIGHTHAND",
    "LEFTSHOULDER",
    "LEFTARM",
    "LEFTFOREARM",
    "LEFTHAND",
    "RIGHTUPLEG",
    "RIGHTLEG",
    "RIGHTFOOT",
    "RIGHTTOEBASE",
    "LEFTUPLEG",
    "LEFTLEG",
    "LEFTFOOT",
    "LEFTTOEBASE",
    "SKATEBOARD_ROOT",
    "TRUCK_FRONT",
    "RIGHT_WHEELFRONT",
    "LEFT_WHEELFRONT",
    "TRUCK_BACK",
    "LEFT_WHEELBACK",
    "RIGHT_WHEELBACK",
    "RIGHTTOEBASE_REPARENTED",
    "LEFTTOEBASE_REPARENTED",
    "RIGHTHAND_REPARENTED",
    "LEFTHAND_REPARENTED",
];

/// The most one skater's state takes on the wire.
pub const MAX_SKATE_STATE_BYTES: usize = 512;
/// The most skater bytes one snapshot carries (some 20 skaters).
pub const MAX_SKATER_FRAME_BYTES: usize = 8 * 1024;
/// How often a skater sends its state.
pub const SEND_INTERVAL_MS: u64 = 50;
/// The longest skate-state label carried.
pub const MAX_LABEL_BYTES: usize = 24;

/// Bone positions are relative to the skater's root, in Skate 3's metres:
/// 1/8192 m steps, so within 4 m of the root.
const BONE_STEPS_PER_METRE: f32 = 8192.0;
/// Velocity on the wire in steps of 2 map units a second.
const VELOCITY_STEP: f32 = 2.0;

const FLAG_ACTIVE: u8 = 1;
const VERSION: u8 = 1;

/// One bone of a skater's pose: where it is relative to the root and how it
/// is turned, in Skate 3's space (metres, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneSample {
    pub offset: Vec3,
    pub rotation: Quat,
}

/// A skater as its own game simulated it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkaterState {
    /// Counts up with each state the skater sends.
    pub seq: u16,
    /// The skater's clock, in milliseconds.
    pub time_ms: u32,
    /// On the board. A state with this off says the skater got off.
    pub active: bool,
    /// The skater's root in map units: where the player stands.
    pub origin: Vec3,
    /// Map units a second.
    pub velocity: Vec3,
    /// The root's turn in the map.
    pub rotation: Quat,
    /// The skate simulation's state, e.g. `PhysicsGround`.
    pub label: String,
    /// `BONES`, each where the pose has it.
    pub bones: Vec<Option<BoneSample>>,
}

impl SkaterState {
    /// Off the board, at `origin`.
    pub fn off(origin: Vec3) -> Self {
        Self {
            seq: 0,
            time_ms: 0,
            active: false,
            origin,
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            label: String::new(),
            bones: Vec::new(),
        }
    }

    /// The root as a matrix in the map.
    pub fn root(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.origin)
    }

    pub fn encode(&self, out: &mut WireWriter) {
        out.put_u8(VERSION);
        out.put_u8(if self.active { FLAG_ACTIVE } else { 0 });
        out.put_u16(self.seq);
        out.put_u32(self.time_ms);
        for v in self.origin.to_array() {
            out.put_f32(v);
        }
        for v in self.velocity.to_array() {
            out.put_u16(quantize(v / VELOCITY_STEP) as u16);
        }
        out.put_u32(pack_quat(self.rotation));
        let label = truncate_label(&self.label);
        out.put_u8(label.len() as u8);
        out.put_bytes(label.as_bytes());
        let mut mask = 0u64;
        for (i, bone) in self.bones.iter().take(BONES.len()).enumerate() {
            if bone.is_some() {
                mask |= 1 << i;
            }
        }
        out.put_u64(mask);
        for bone in self.bones.iter().take(BONES.len()).flatten() {
            for v in bone.offset.to_array() {
                out.put_u16(quantize(v * BONE_STEPS_PER_METRE) as u16);
            }
            out.put_u32(pack_quat(bone.rotation));
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = WireWriter::with_capacity(MAX_SKATE_STATE_BYTES);
        self.encode(&mut out);
        out.finish()
    }

    pub fn decode(input: &mut WireReader<'_>) -> Result<Self, WireError> {
        if input.get_u8()? != VERSION {
            return Err(WireError::Malformed("unknown skater state version"));
        }
        let flags = input.get_u8()?;
        let seq = input.get_u16()?;
        let time_ms = input.get_u32()?;
        let mut origin = [0.0; 3];
        for v in &mut origin {
            *v = input.get_f32()?;
        }
        let mut velocity = [0.0; 3];
        for v in &mut velocity {
            *v = f32::from(input.get_u16()? as i16) * VELOCITY_STEP;
        }
        let rotation = unpack_quat(input.get_u32()?);
        let len = usize::from(input.get_u8()?);
        if len > MAX_LABEL_BYTES {
            return Err(WireError::Malformed("skater label too long"));
        }
        let mut label = vec![0; len];
        input.get_bytes(&mut label)?;
        let label = String::from_utf8(label)
            .map_err(|_| WireError::Malformed("skater label is not UTF-8"))?;
        let mask = input.get_u64()?;
        if mask >> BONES.len() != 0 {
            return Err(WireError::Malformed("skater bone outside the rig"));
        }
        let mut bones = Vec::with_capacity(if mask == 0 { 0 } else { BONES.len() });
        if mask != 0 {
            for i in 0..BONES.len() {
                if mask & (1 << i) == 0 {
                    bones.push(None);
                    continue;
                }
                let mut offset = [0.0; 3];
                for v in &mut offset {
                    *v = f32::from(input.get_u16()? as i16) / BONE_STEPS_PER_METRE;
                }
                bones.push(Some(BoneSample {
                    offset: Vec3::from_array(offset),
                    rotation: unpack_quat(input.get_u32()?),
                }));
            }
        }
        let origin = Vec3::from_array(origin);
        if !origin.is_finite() || origin.abs().max_element() > 1.0e6 {
            return Err(WireError::Malformed("skater origin out of range"));
        }
        Ok(Self {
            seq,
            time_ms,
            active: flags & FLAG_ACTIVE != 0,
            origin,
            velocity: Vec3::from_array(velocity),
            rotation,
            label,
            bones,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_SKATE_STATE_BYTES {
            return Err(WireError::Malformed("skater state too long"));
        }
        let mut input = WireReader::new(bytes);
        let state = Self::decode(&mut input)?;
        if !input.is_empty() {
            return Err(WireError::Malformed("trailing bytes after a skater state"));
        }
        Ok(state)
    }
}

fn truncate_label(label: &str) -> &str {
    let mut end = label.len().min(MAX_LABEL_BYTES);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    &label[..end]
}

fn quantize(v: f32) -> i16 {
    if v.is_finite() {
        v.round().clamp(-32767.0, 32767.0) as i16
    } else {
        0
    }
}

/// A unit quaternion in 32 bits: which component is largest (2 bits), and
/// the other three in 10 bits each, the largest made positive.
pub fn pack_quat(q: Quat) -> u32 {
    let q = if q.is_finite() && q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    let v = q.to_array();
    let largest = (0..4)
        .max_by(|&a, &b| v[a].abs().total_cmp(&v[b].abs()))
        .unwrap_or(3);
    let sign = if v[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut bits = largest as u32;
    let mut shift = 2;
    for (i, c) in v.iter().enumerate() {
        if i == largest {
            continue;
        }
        let n = ((c * sign * std::f32::consts::SQRT_2 * 0.5 + 0.5) * 1023.0)
            .round()
            .clamp(0.0, 1023.0) as u32;
        bits |= n << shift;
        shift += 10;
    }
    bits
}

pub fn unpack_quat(bits: u32) -> Quat {
    let largest = (bits & 3) as usize;
    let mut v = [0.0f32; 4];
    let mut shift = 2;
    let mut sum = 0.0;
    for (i, c) in v.iter_mut().enumerate() {
        if i == largest {
            continue;
        }
        let n = ((bits >> shift) & 1023) as f32 / 1023.0;
        *c = (n - 0.5) * 2.0 / std::f32::consts::SQRT_2;
        sum += *c * *c;
        shift += 10;
    }
    v[largest] = (1.0 - sum).max(0.0).sqrt();
    Quat::from_array(v).normalize()
}

/// Bone matrices as Skate 3 gives them (`names` beside them) to the wire's
/// samples: positions and turns, relative to the skater's root.
pub fn sample_bones(names: &[String], bones: &[Mat4]) -> Vec<Option<BoneSample>> {
    BONES
        .iter()
        .map(|wanted| {
            let i = names.iter().position(|n| n == wanted)?;
            let m = bones.get(i)?;
            let (scale, rotation, offset) = m.to_scale_rotation_translation();
            (scale.is_finite() && rotation.is_finite() && offset.is_finite()).then_some(
                BoneSample {
                    offset,
                    rotation: rotation.normalize(),
                },
            )
        })
        .collect()
}

/// The wire's samples back to bone matrices, in `BONES` order (a bone the
/// pose lacks is the identity).
pub fn bone_matrices(bones: &[Option<BoneSample>]) -> Vec<Mat4> {
    (0..BONES.len())
        .map(|i| {
            bones.get(i).copied().flatten().map_or(Mat4::IDENTITY, |b| {
                Mat4::from_rotation_translation(b.rotation, b.offset)
            })
        })
        .collect()
}

/// The furthest a bone moves (in millimetres) from where the pose has it to
/// where the wire puts it: the price of the quantizing.
pub fn round_trip_error_mm(names: &[String], bones: &[Mat4]) -> f32 {
    let sampled = sample_bones(names, bones);
    let state = SkaterState {
        bones: sampled,
        ..SkaterState::off(Vec3::ZERO)
    };
    let Ok(back) = SkaterState::from_bytes(&state.to_bytes()) else {
        return f32::INFINITY;
    };
    let mut worst = 0.0f32;
    for (wanted, sample) in BONES.iter().zip(back.bones.iter()) {
        let (Some(i), Some(sample)) = (names.iter().position(|n| n == wanted), sample) else {
            continue;
        };
        let Some(m) = bones.get(i) else { continue };
        let back = Mat4::from_rotation_translation(sample.rotation, sample.offset);
        // A point a hand's length from the joint, as a mesh vertex would be.
        for p in [Vec3::ZERO, Vec3::X * 0.2, Vec3::Y * 0.2, Vec3::Z * 0.2] {
            worst = worst.max((m.transform_point3(p) - back.transform_point3(p)).length());
        }
    }
    worst * 1000.0
}

/// Skater states for one snapshot, by client.
pub fn encode_frame(states: &[(u32, Vec<u8>)]) -> Vec<u8> {
    encode_frame_marked(states).0
}

/// `encode_frame`, and which of `states` fitted.
fn encode_frame_marked(states: &[(u32, Vec<u8>)]) -> (Vec<u8>, Vec<bool>) {
    let mut out = WireWriter::new();
    let mut count = 0u8;
    let mut fitted = vec![false; states.len()];
    out.put_u8(0);
    for (i, (client, bytes)) in states.iter().enumerate() {
        if count == u8::MAX
            || bytes.len() > MAX_SKATE_STATE_BYTES
            || out.len() + 6 + bytes.len() > MAX_SKATER_FRAME_BYTES
        {
            continue;
        }
        out.put_u32(*client);
        out.put_u16(bytes.len() as u16);
        out.put_bytes(bytes);
        fitted[i] = true;
        count += 1;
    }
    if count == 0 {
        return (Vec::new(), fitted);
    }
    let mut bytes = out.finish();
    bytes[0] = count;
    (bytes, fitted)
}

pub fn decode_frame(bytes: &[u8]) -> Result<Vec<(u32, SkaterState)>, WireError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.len() > MAX_SKATER_FRAME_BYTES {
        return Err(WireError::Malformed("skater frame too long"));
    }
    let mut input = WireReader::new(bytes);
    let count = input.get_u8()?;
    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let client = input.get_u32()?;
        let len = usize::from(input.get_u16()?);
        if len > MAX_SKATE_STATE_BYTES {
            return Err(WireError::Malformed("skater state too long"));
        }
        let mut state = vec![0; len];
        input.get_bytes(&mut state)?;
        out.push((client, SkaterState::from_bytes(&state)?));
    }
    if !input.is_empty() {
        return Err(WireError::Malformed("trailing bytes after the skaters"));
    }
    Ok(out)
}

/// What the host does with a skater's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Move the player there.
    Apply,
    /// The skater got off (or may no longer skate): give the player back
    /// its own movement.
    Stop,
    /// Leave the player where it is.
    Reject(Refusal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// An old state, arriving after a newer one.
    Stale,
    /// The player is dead or not in the match.
    NotAlive,
    /// Got on the board away from where the player stands.
    StartedElsewhere,
    /// Moved further than a skater can since the last state.
    TooFast,
    /// Not on the board, and was not.
    Idle,
}

/// How the host checks one client's skating.
#[derive(Clone, Debug, Default)]
pub struct SkaterGate {
    /// On the board, as the host sees it.
    pub active: bool,
    last_seq: Option<u16>,
    /// Where and when (host seconds) the last applied state put the player.
    last: Option<(Vec3, f64)>,
    pub accepted: u64,
    pub rejected: u64,
}

impl SkaterGate {
    /// The furthest from where the player stands that it may get on.
    pub const START_RADIUS: f32 = 128.0;
    /// Map units a second: a skater falling or bombing a hill, with room.
    pub const MAX_SPEED: f32 = 1600.0;
    /// Seconds of arrival jitter allowed for.
    pub const JITTER_SECS: f32 = 0.25;
    /// Map units allowed on top, whatever the time.
    pub const SLACK: f32 = 48.0;

    /// Judges `state`, which arrived at host time `now` (seconds), for a
    /// player standing at `player` (alive or not).
    pub fn judge(&mut self, state: &SkaterState, now: f64, player: Vec3, alive: bool) -> Verdict {
        if let Some(last) = self.last_seq
            && !seq_after(state.seq, last)
        {
            return Verdict::Reject(Refusal::Stale);
        }
        self.last_seq = Some(state.seq);
        if !alive || !state.active {
            let was = self.active;
            self.active = false;
            self.last = None;
            return if was {
                Verdict::Stop
            } else if !alive {
                self.rejected += 1;
                Verdict::Reject(Refusal::NotAlive)
            } else {
                Verdict::Reject(Refusal::Idle)
            };
        }
        match self.last {
            None => {
                if state.origin.distance(player) > Self::START_RADIUS {
                    self.rejected += 1;
                    return Verdict::Reject(Refusal::StartedElsewhere);
                }
            }
            Some((at, then)) => {
                let secs = (now - then).max(0.0) as f32 + Self::JITTER_SECS;
                if state.origin.distance(at) > Self::MAX_SPEED * secs + Self::SLACK {
                    self.rejected += 1;
                    return Verdict::Reject(Refusal::TooFast);
                }
            }
        }
        self.active = true;
        self.last = Some((state.origin, now));
        self.accepted += 1;
        Verdict::Apply
    }

    /// Seconds since the last applied state, if any.
    pub fn silent_for(&self, now: f64) -> Option<f64> {
        self.last.map(|(_, then)| now - then)
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.last = None;
    }
}

fn seq_after(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

/// A skater the host knows of.
#[derive(Debug, Default)]
pub struct RelayedSkater {
    pub gate: SkaterGate,
    /// The newest state, encoded as it goes to others, and its sequence.
    pub latest: Option<(u16, Vec<u8>)>,
    pub latest_state: Option<SkaterState>,
    /// The host's own player, simulated here.
    pub local: bool,
    /// Bytes of this skater's states the host received, and sent to others.
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub last_refusal: Option<Refusal>,
}

/// The host's skaters: checked, applied, and passed on.
#[derive(Resource, Debug, Default)]
pub struct SkaterRelay {
    pub skaters: BTreeMap<u32, RelayedSkater>,
    /// For each peer, the newest sequence of each skater it was sent.
    sent: HashMap<u32, HashMap<u32, u16>>,
    /// States that arrived from clients, not yet judged.
    pub inbound: Vec<(u32, Vec<u8>)>,
    /// Snapshot bytes of skater states sent, all peers together.
    pub frame_bytes: u64,
}

impl SkaterRelay {
    /// The skater states `peer` has not had yet: every other skater's
    /// newest (an off state included, so it stops drawing it).
    pub fn frame_for(&mut self, peer: u32) -> Vec<u8> {
        let sent = self.sent.entry(peer).or_default();
        let mut states = Vec::new();
        for (client, skater) in &self.skaters {
            if *client == peer {
                continue;
            }
            let Some((seq, bytes)) = &skater.latest else {
                continue;
            };
            if sent.get(client) == Some(seq) {
                continue;
            }
            states.push((*client, bytes.clone()));
        }
        let (frame, fitted) = encode_frame_marked(&states);
        for ((client, bytes), _) in states.iter().zip(&fitted).filter(|(_, f)| **f) {
            if let Some(skater) = self.skaters.get_mut(client) {
                if let Some((seq, _)) = &skater.latest {
                    sent.insert(*client, *seq);
                }
                skater.bytes_out += bytes.len() as u64 + 6;
            }
        }
        self.frame_bytes += frame.len() as u64;
        frame
    }

    /// Forgets a client that left.
    pub fn retire(&mut self, client: u32) {
        self.skaters.remove(&client);
        self.sent.remove(&client);
        for sent in self.sent.values_mut() {
            sent.remove(&client);
        }
    }

    pub fn skating(&self, client: u32) -> bool {
        self.skaters.get(&client).is_some_and(|s| s.gate.active)
    }
}

/// The local skater's newest state, for the host (its own player) or the
/// client (to send).
#[derive(Resource, Debug, Default)]
pub struct SkateOutbox {
    /// The newest state not yet taken.
    pub pending: Option<SkaterState>,
    next_seq: u16,
    started: Option<std::time::Instant>,
    last_sent: Option<std::time::Instant>,
    /// Off states still to send, after getting off (they may be lost).
    off_repeats: u8,
    last_off: Option<SkaterState>,
    pub sent: u64,
    pub sent_bytes: u64,
    /// Bytes sent over the last full second, and the second's start.
    pub bytes_per_s: u64,
    window: (Option<std::time::Instant>, u64),
    /// How far quantizing moves a bone, at worst so far (mm).
    pub worst_error_mm: f32,
}

impl SkateOutbox {
    /// Offers the local skater's state; `seq` and `time_ms` are filled in.
    pub fn publish(&mut self, mut state: SkaterState) {
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        state.time_ms = started.elapsed().as_millis() as u32;
        if !state.active {
            self.off_repeats = 3;
        }
        self.pending = Some(state);
    }

    /// The state to send now, if one is due: at most every
    /// `SEND_INTERVAL_MS`, an off state at once and a few times over.
    pub fn take_due(&mut self, now: std::time::Instant) -> Option<SkaterState> {
        let due = self
            .last_sent
            .is_none_or(|t| now.duration_since(t).as_millis() as u64 >= SEND_INTERVAL_MS);
        let mut state = match self.pending.take() {
            Some(state) if !state.active => state,
            Some(state) if due => state,
            Some(state) => {
                self.pending = Some(state);
                return None;
            }
            None if due && self.off_repeats > 0 => self.last_off.clone()?,
            None => return None,
        };
        state.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        if !state.active {
            self.off_repeats = self.off_repeats.saturating_sub(1);
            self.last_off = Some(state.clone());
        } else {
            self.off_repeats = 0;
            self.last_off = None;
        }
        self.last_sent = Some(now);
        Some(state)
    }

    /// Counts `bytes` sent at `now`.
    pub fn note_sent(&mut self, bytes: usize, now: std::time::Instant) {
        self.sent += 1;
        self.sent_bytes += bytes as u64;
        let (start, count) = &mut self.window;
        match start {
            Some(s) if now.duration_since(*s).as_secs_f32() < 1.0 => *count += bytes as u64,
            _ => {
                if start.is_some() {
                    self.bytes_per_s = *count;
                }
                *start = Some(now);
                *count = bytes as u64;
            }
        }
    }
}

/// Skater states a client took off snapshots, in order.
#[derive(Resource, Debug, Default)]
pub struct SkaterDownlink {
    pub frames: Vec<Vec<u8>>,
    pub bytes: u64,
}

/// How far behind the newest state remote skaters are drawn, for room to
/// interpolate.
pub const INTERP_DELAY_MS: f64 = 100.0;
/// A remote skater with no state for this long has stopped.
pub const SILENT_MS: f64 = 1500.0;

/// One remote skater's recent states, to draw between.
#[derive(Debug, Default)]
pub struct SkaterTrack {
    /// (local ms received, state), oldest first.
    samples: VecDeque<(f64, SkaterState)>,
    /// The smallest (local receive - skater clock) seen: the clock offset
    /// with the least delay in it.
    offset: Option<f64>,
    pub received: u64,
}

impl SkaterTrack {
    const KEEP: usize = 16;

    pub fn push(&mut self, local_ms: f64, state: SkaterState) {
        if self
            .samples
            .back()
            .is_some_and(|(_, last)| !seq_after(state.seq, last.seq))
        {
            return;
        }
        let offset = local_ms - f64::from(state.time_ms);
        // A skater's clock restarting (a new game) resets the offset.
        self.offset = Some(match self.offset {
            Some(o) if (offset - o).abs() < 10_000.0 => o.min(offset),
            _ => offset,
        });
        self.received += 1;
        self.samples.push_back((local_ms, state));
        while self.samples.len() > Self::KEEP {
            self.samples.pop_front();
        }
    }

    pub fn newest(&self) -> Option<&SkaterState> {
        self.samples.back().map(|(_, s)| s)
    }

    pub fn silent(&self, local_ms: f64) -> bool {
        self.samples
            .back()
            .is_none_or(|(at, _)| local_ms - at > SILENT_MS)
    }

    /// The skater as it was `INTERP_DELAY_MS` ago, between the states
    /// either side; `None` when it is off its board.
    pub fn sample(&self, local_ms: f64) -> Option<SkaterState> {
        let newest = self.newest()?;
        if !newest.active {
            return None;
        }
        let offset = self.offset?;
        let want = local_ms - offset - INTERP_DELAY_MS;
        let mut before: Option<&SkaterState> = None;
        for (_, state) in &self.samples {
            if !state.active {
                continue;
            }
            if f64::from(state.time_ms) <= want {
                before = Some(state);
                continue;
            }
            let Some(a) = before else {
                return Some(state.clone());
            };
            let span = f64::from(state.time_ms.wrapping_sub(a.time_ms)).max(1.0);
            let t = ((want - f64::from(a.time_ms)) / span).clamp(0.0, 1.0) as f32;
            return Some(blend(a, state, t));
        }
        before.cloned()
    }
}

/// `a` to `b` by `t`.
pub fn blend(a: &SkaterState, b: &SkaterState, t: f32) -> SkaterState {
    let bones = if a.bones.len() == b.bones.len() {
        a.bones
            .iter()
            .zip(&b.bones)
            .map(|(x, y)| match (x, y) {
                (Some(x), Some(y)) => Some(BoneSample {
                    offset: x.offset.lerp(y.offset, t),
                    rotation: x.rotation.slerp(y.rotation, t),
                }),
                (_, y) => *y,
            })
            .collect()
    } else {
        b.bones.clone()
    };
    SkaterState {
        seq: b.seq,
        time_ms: b.time_ms,
        active: b.active,
        origin: a.origin.lerp(b.origin, t),
        velocity: a.velocity.lerp(b.velocity, t),
        rotation: a.rotation.slerp(b.rotation, t),
        label: b.label.clone(),
        bones,
    }
}

/// Every remote skater this game draws, as received.
#[derive(Resource, Debug, Default)]
pub struct SkaterTracks {
    pub tracks: BTreeMap<u32, SkaterTrack>,
}

impl SkaterTracks {
    pub fn push(&mut self, client: u32, local_ms: f64, state: SkaterState) {
        self.tracks.entry(client).or_default().push(local_ms, state);
    }

    /// Drops skaters gone silent or off their boards.
    pub fn prune(&mut self, local_ms: f64) {
        self.tracks
            .retain(|_, t| !t.silent(local_ms) && t.newest().is_some_and(|s| s.active));
    }
}

/// The host's side, each authority tick before the step: judges the
/// skater states clients sent and moves their players; takes its own
/// player's (which the local skate mode already moved); stops skaters gone
/// quiet, dead or gone.
pub fn skate_authority(
    hub: Option<bevy::prelude::ResMut<crate::transport::udp_session::UdpAuthorityHub>>,
    mut relay: bevy::prelude::ResMut<SkaterRelay>,
    mut outbox: bevy::prelude::ResMut<SkateOutbox>,
    local: Option<bevy::prelude::Res<crate::LocalPresentClient>>,
    role: bevy::prelude::Res<frame::RuntimeRole>,
    mut world: bevy::prelude::ResMut<crate::AuthorityWorld>,
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>,
) {
    let now = time.elapsed_secs_f64();
    let instant = std::time::Instant::now();
    let mut inbound = std::mem::take(&mut relay.inbound);
    if let Some(mut hub) = hub {
        inbound.extend(hub.take_skate_inbound());
    }
    let local = local
        .filter(|_| *role != frame::RuntimeRole::Dedicated)
        .map(|l| l.0.0);
    for (client, bytes) in inbound {
        if Some(client) == local {
            continue;
        }
        let Ok(state) = SkaterState::from_bytes(&bytes) else {
            continue;
        };
        let id = sim::ClientId(client);
        let Some(ps) = world.0.player(id) else {
            continue;
        };
        let (player, alive) = (Vec3::from_array(ps.origin), ps.pm_type == 0);
        let skater = relay.skaters.entry(client).or_default();
        skater.bytes_in += bytes.len() as u64;
        match skater.gate.judge(&state, now, player, alive) {
            Verdict::Apply => {
                // Every time: a hold (the Minecraft map waiting for ground)
                // may have handed the player its own movement back.
                world.0.set_external_motion(id, true);
                world.0.set_origin(id, state.origin.to_array());
                skater.latest = Some((state.seq, bytes));
                skater.latest_state = Some(state);
            }
            Verdict::Stop => {
                world.0.set_external_motion(id, false);
                let off = SkaterState::off(state.origin);
                skater.latest = Some((
                    state.seq,
                    SkaterState {
                        seq: state.seq,
                        ..off.clone()
                    }
                    .to_bytes(),
                ));
                skater.latest_state = Some(off);
                diag::info!(Net, "skater {client} got off");
            }
            Verdict::Reject(why) => {
                if skater.last_refusal != Some(why) {
                    diag::info!(Net, "skater {client}: state refused ({why:?})");
                }
                skater.last_refusal = Some(why);
            }
        }
    }
    // The host's own player, already moved by its skate mode.
    if let Some(local) = local
        && let Some(state) = outbox.take_due(instant)
    {
        let bytes = state.to_bytes();
        outbox.note_sent(bytes.len(), instant);
        let skater = relay.skaters.entry(local).or_default();
        skater.local = true;
        skater.gate.active = state.active;
        skater.latest = Some((state.seq, bytes));
        skater.latest_state = Some(state);
    }
    let mut gone = Vec::new();
    for (client, skater) in &mut relay.skaters {
        if skater.local {
            continue;
        }
        let id = sim::ClientId(*client);
        let Some((origin, pm_type)) = world.0.player(id).map(|ps| (ps.origin, ps.pm_type)) else {
            gone.push(*client);
            continue;
        };
        let quiet = skater
            .gate
            .silent_for(now)
            .is_some_and(|secs| secs * 1000.0 > SILENT_MS);
        if skater.gate.active && (quiet || pm_type != 0) {
            skater.gate.stop();
            world.0.set_external_motion(id, false);
            let seq = skater
                .latest
                .as_ref()
                .map_or(0, |(seq, _)| seq.wrapping_add(1));
            let off = SkaterState {
                seq,
                ..SkaterState::off(Vec3::from_array(origin))
            };
            skater.latest = Some((seq, off.to_bytes()));
            skater.latest_state = Some(off);
            diag::info!(
                Net,
                "skater {client} stopped: {}",
                if quiet { "no states" } else { "not alive" }
            );
        }
    }
    for client in gone {
        relay.retire(client);
    }
}

/// The client's side: sends its own skater's state when one is due.
pub fn send_skate_uplink(
    link: Option<bevy::prelude::ResMut<crate::transport::udp_session::UdpClientLink>>,
    mut outbox: bevy::prelude::ResMut<SkateOutbox>,
) {
    let Some(mut link) = link else {
        return;
    };
    if link.connection.is_none() || !link.has_entered_match() {
        return;
    }
    let now = std::time::Instant::now();
    let Some(state) = outbox.take_due(now) else {
        return;
    };
    match link.send_skate(state.to_bytes()) {
        Ok(bytes) => outbox.note_sent(bytes, now),
        Err(e) => diag::warn!(Net, "skate send: {e}"),
    }
}

/// Everyone's side: the other skaters, drawn between the states the host
/// passed on (on the host, straight from its relay).
pub fn update_remote_skaters(
    relay: Option<bevy::prelude::Res<SkaterRelay>>,
    mut downlink: bevy::prelude::ResMut<SkaterDownlink>,
    mut tracks: bevy::prelude::ResMut<SkaterTracks>,
    mut remote: bevy::prelude::ResMut<frame::RemoteSkaters>,
    local: bevy::prelude::Res<crate::LocalPresentClient>,
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>,
) {
    let now_ms = time.elapsed_secs_f64() * 1000.0;
    if let Some(relay) = relay {
        for (client, skater) in &relay.skaters {
            if skater.local {
                continue;
            }
            if let Some(state) = &skater.latest_state {
                let mut state = state.clone();
                if let Some((seq, _)) = &skater.latest {
                    state.seq = *seq;
                }
                tracks.push(*client, now_ms, state);
            }
        }
    }
    for frame in std::mem::take(&mut downlink.frames) {
        match decode_frame(&frame) {
            Ok(states) => {
                for (client, state) in states {
                    tracks.push(client, now_ms, state);
                }
            }
            Err(e) => diag::warn!(Net, "skater frame: {e}"),
        }
    }
    tracks.prune(now_ms);
    if remote.names.is_empty() {
        remote.names = BONES.iter().map(|n| (*n).to_owned()).collect();
    }
    remote.poses.clear();
    for (client, track) in &tracks.tracks {
        if *client == local.0.0 {
            continue;
        }
        let Some(state) = track.sample(now_ms) else {
            continue;
        };
        remote.poses.insert(
            *client,
            frame::SkaterPose {
                root: state.root(),
                bones: bone_matrices(&state.bones),
                label: state.label,
                received: track.received,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        std::iter::once("TRAJECTORY")
            .chain(BONES)
            .map(str::to_owned)
            .collect()
    }

    fn pose(turn: f32) -> Vec<Mat4> {
        (0..=BONES.len())
            .map(|i| {
                let f = i as f32;
                Mat4::from_rotation_translation(
                    Quat::from_euler(
                        bevy::math::EulerRot::YXZ,
                        turn + f * 0.3,
                        f * 0.05,
                        -f * 0.02,
                    ),
                    Vec3::new(f * 0.03 - 0.5, f * 0.04, 0.2 - f * 0.01),
                )
            })
            .collect()
    }

    fn state(seq: u16, time_ms: u32, origin: Vec3) -> SkaterState {
        SkaterState {
            seq,
            time_ms,
            active: true,
            origin,
            velocity: Vec3::new(300.0, -20.0, 0.0),
            rotation: Quat::from_rotation_z(0.7),
            label: "PhysicsGround".into(),
            bones: sample_bones(&names(), &pose(0.0)),
        }
    }

    #[test]
    fn a_state_survives_the_wire() {
        let s = state(7, 1234, Vec3::new(11520.5, -36.0, 2214.25));
        let bytes = s.to_bytes();
        let back = SkaterState::from_bytes(&bytes).unwrap();
        assert_eq!(back.seq, 7);
        assert_eq!(back.time_ms, 1234);
        assert!(back.active);
        assert_eq!(back.origin, s.origin);
        assert!(back.velocity.distance(s.velocity) <= 2.0);
        assert!(back.rotation.angle_between(s.rotation) < 0.003);
        assert_eq!(back.label, "PhysicsGround");
        assert_eq!(back.bones.iter().flatten().count(), BONES.len());
    }

    #[test]
    fn a_full_pose_is_some_four_hundred_bytes() {
        let bytes = state(1, 0, Vec3::ZERO).to_bytes();
        assert!(bytes.len() <= 410, "{} bytes", bytes.len());
        // header 1+1+2+4+12+6+4, label 1+13, mask 8, 35 bones of 10 bytes
        assert_eq!(bytes.len(), 30 + 14 + 8 + BONES.len() * 10);
    }

    #[test]
    fn quantized_bones_stay_within_a_millimetre() {
        let err = round_trip_error_mm(&names(), &pose(1.1));
        assert!(err < 1.0, "{err} mm");
    }

    #[test]
    fn quaternions_pack_to_a_tenth_of_a_degree() {
        for i in 0..200 {
            let f = i as f32;
            let q = Quat::from_euler(bevy::math::EulerRot::XYZ, f * 0.37, -f * 0.11, f * 0.53);
            let back = unpack_quat(pack_quat(q));
            assert!(
                back.angle_between(q).to_degrees() < 0.2,
                "{q:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn an_off_state_carries_no_pose() {
        let off = SkaterState::off(Vec3::new(1.0, 2.0, 3.0));
        let back = SkaterState::from_bytes(&off.to_bytes()).unwrap();
        assert!(!back.active);
        assert!(back.bones.is_empty());
        assert!(off.to_bytes().len() < 40);
    }

    #[test]
    fn junk_is_refused() {
        let bytes = state(1, 0, Vec3::ZERO).to_bytes();
        assert!(SkaterState::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(SkaterState::from_bytes(&longer).is_err());
        let mut wrong = bytes.clone();
        wrong[0] = 9;
        assert!(SkaterState::from_bytes(&wrong).is_err());
        let mut far = state(1, 0, Vec3::new(f32::NAN, 0.0, 0.0)).to_bytes();
        assert!(SkaterState::from_bytes(&far).is_err());
        far = state(1, 0, Vec3::new(5.0e7, 0.0, 0.0)).to_bytes();
        assert!(SkaterState::from_bytes(&far).is_err());
        assert!(decode_frame(&[3, 0, 0]).is_err());
    }

    #[test]
    fn frames_carry_several_skaters_and_stay_bounded() {
        let one = state(1, 0, Vec3::ZERO).to_bytes();
        let frame = encode_frame(&[(1, one.clone()), (4, one.clone())]);
        let back = decode_frame(&frame).unwrap();
        assert_eq!(back.iter().map(|(c, _)| *c).collect::<Vec<_>>(), [1, 4]);
        let many: Vec<_> = (0..100).map(|c| (c, one.clone())).collect();
        let frame = encode_frame(&many);
        assert!(frame.len() <= MAX_SKATER_FRAME_BYTES);
        assert_eq!(
            decode_frame(&frame).unwrap().len(),
            MAX_SKATER_FRAME_BYTES / (one.len() + 6)
        );
        assert!(encode_frame(&[]).is_empty());
        assert!(decode_frame(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_gate_takes_a_skater_who_starts_where_it_stands() {
        let mut gate = SkaterGate::default();
        let at = Vec3::new(100.0, 0.0, 40.0);
        assert_eq!(
            gate.judge(&state(1, 0, at + Vec3::Z * 2.0), 0.0, at, true),
            Verdict::Apply
        );
        // 8.5 m/s, a pushing skater, for 50 ms.
        let step = Vec3::X * 8.5 / 0.0254 * 0.05;
        assert_eq!(
            gate.judge(&state(2, 50, at + step), 0.05, at, true),
            Verdict::Apply
        );
        assert!(gate.active);
        assert_eq!(gate.accepted, 2);
    }

    #[test]
    fn the_gate_refuses_a_start_far_away_and_teleports() {
        let mut gate = SkaterGate::default();
        let at = Vec3::ZERO;
        assert_eq!(
            gate.judge(&state(1, 0, Vec3::X * 500.0), 0.0, at, true),
            Verdict::Reject(Refusal::StartedElsewhere)
        );
        assert_eq!(gate.judge(&state(2, 0, at), 0.0, at, true), Verdict::Apply);
        assert_eq!(
            gate.judge(&state(3, 50, Vec3::X * 3000.0), 0.05, at, true),
            Verdict::Reject(Refusal::TooFast)
        );
        // Still where it was: the next honest state is taken.
        assert_eq!(
            gate.judge(&state(4, 100, Vec3::X * 30.0), 0.1, at, true),
            Verdict::Apply
        );
        // After a second of lost packets it may be far.
        assert_eq!(
            gate.judge(&state(5, 1100, Vec3::X * 1000.0), 1.1, at, true),
            Verdict::Apply
        );
        assert_eq!(gate.rejected, 2);
    }

    #[test]
    fn the_gate_drops_old_states_and_stops_the_dead() {
        let mut gate = SkaterGate::default();
        assert_eq!(
            gate.judge(&state(10, 0, Vec3::ZERO), 0.0, Vec3::ZERO, true),
            Verdict::Apply
        );
        assert_eq!(
            gate.judge(&state(9, 0, Vec3::ZERO), 0.0, Vec3::ZERO, true),
            Verdict::Reject(Refusal::Stale)
        );
        assert_eq!(
            gate.judge(&state(11, 50, Vec3::ZERO), 0.05, Vec3::ZERO, false),
            Verdict::Stop
        );
        assert!(!gate.active);
        assert_eq!(
            gate.judge(&state(12, 100, Vec3::ZERO), 0.1, Vec3::ZERO, false),
            Verdict::Reject(Refusal::NotAlive)
        );
        let mut off = SkaterState::off(Vec3::ZERO);
        off.seq = 13;
        assert_eq!(
            gate.judge(&off, 0.2, Vec3::ZERO, true),
            Verdict::Reject(Refusal::Idle)
        );
        // Sequence numbers wrap.
        let mut gate = SkaterGate::default();
        assert_eq!(
            gate.judge(&state(65535, 0, Vec3::ZERO), 0.0, Vec3::ZERO, true),
            Verdict::Apply
        );
        assert_eq!(
            gate.judge(&state(0, 50, Vec3::ZERO), 0.05, Vec3::ZERO, true),
            Verdict::Apply
        );
    }

    #[test]
    fn the_relay_sends_each_state_once_and_not_back() {
        let mut relay = SkaterRelay::default();
        let bytes = state(1, 0, Vec3::ZERO).to_bytes();
        relay.skaters.insert(
            1,
            RelayedSkater {
                latest: Some((1, bytes.clone())),
                ..Default::default()
            },
        );
        assert!(
            relay.frame_for(1).is_empty(),
            "its own state back to the skater"
        );
        let frame = relay.frame_for(2);
        assert_eq!(decode_frame(&frame).unwrap().len(), 1);
        assert!(relay.frame_for(2).is_empty(), "the same state again");
        relay.skaters.get_mut(&1).unwrap().latest = Some((2, bytes));
        assert_eq!(decode_frame(&relay.frame_for(2)).unwrap().len(), 1);
        assert_eq!(
            relay.skaters[&1].bytes_out,
            2 * (state(1, 0, Vec3::ZERO).to_bytes().len() as u64 + 6)
        );
        relay.retire(1);
        assert!(relay.frame_for(2).is_empty());
    }

    #[test]
    fn the_outbox_paces_states_and_repeats_getting_off() {
        let mut outbox = SkateOutbox::default();
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        outbox.publish(state(0, 0, Vec3::ZERO));
        assert_eq!(outbox.take_due(ms(0)).unwrap().seq, 0);
        outbox.publish(state(0, 0, Vec3::ZERO));
        assert!(outbox.take_due(ms(20)).is_none(), "sooner than 50 ms");
        assert_eq!(outbox.take_due(ms(60)).unwrap().seq, 1);
        outbox.publish(SkaterState::off(Vec3::ZERO));
        let off = outbox.take_due(ms(70)).unwrap();
        assert!(!off.active, "getting off goes at once");
        assert!(outbox.take_due(ms(80)).is_none());
        assert!(!outbox.take_due(ms(130)).unwrap().active);
        assert!(!outbox.take_due(ms(190)).unwrap().active);
        assert!(
            outbox.take_due(ms(250)).is_none(),
            "three off states, then quiet"
        );
    }

    #[test]
    fn remote_skaters_are_drawn_between_states() {
        let mut track = SkaterTrack::default();
        // The skater's clock starts 5 s behind ours; states 50 ms apart.
        for i in 0..6u16 {
            let t = u32::from(i) * 50;
            track.push(
                5000.0 + f64::from(t) + 20.0,
                state(i, t, Vec3::X * f32::from(i) * 10.0),
            );
        }
        // Now 5270: drawn at skater time 5270 - 5020 - 100 = 150 -> state 3.
        let at = track.sample(5270.0).unwrap();
        assert!((at.origin.x - 30.0).abs() < 0.01, "{:?}", at.origin);
        // 25 ms later: halfway to state 4.
        let at = track.sample(5295.0).unwrap();
        assert!((at.origin.x - 35.0).abs() < 0.01, "{:?}", at.origin);
        // Past the newest: held at it.
        assert!((track.sample(9000.0).unwrap().origin.x - 50.0).abs() < 0.01);
        // Getting off ends it at once.
        let mut off = SkaterState::off(Vec3::ZERO);
        off.seq = 6;
        off.time_ms = 300;
        track.push(5320.0, off);
        assert!(track.sample(5330.0).is_none());
        let mut tracks = SkaterTracks::default();
        tracks.tracks.insert(3, track);
        tracks.prune(5330.0);
        assert!(tracks.tracks.is_empty());
    }

    #[test]
    fn a_snapshot_carries_skaters_with_or_without_minecraft_bytes() {
        use crate::transport::frame::{Frame, frame_from_acked_tick};
        let snapshot = sim::SimWorld::new().snapshot(sim::Tick(3));
        let skaters = encode_frame(&[(2, state(5, 250, Vec3::new(1.0, 2.0, 3.0)).to_bytes())]);
        for (minecraft, skaters) in [
            (vec![], skaters.clone()),
            (vec![7, 8, 9], skaters.clone()),
            (vec![7, 8, 9], vec![]),
            (vec![], vec![]),
        ] {
            let mut encoder = crate::SnapshotEncoder::new();
            let mut frame =
                frame_from_acked_tick(&mut encoder, &sim::TickInput::default(), &snapshot, vec![]);
            frame.minecraft = minecraft.clone();
            frame.skaters = skaters.clone();
            let bytes = frame.to_bytes();
            let back = Frame::decode(
                &mut WireReader::new(&bytes),
                &mut crate::WorldObjectSyncDecoder::default(),
            )
            .unwrap();
            assert_eq!(back.minecraft, minecraft);
            assert_eq!(back.skaters, skaters);
        }
        let back = decode_frame(&skaters).unwrap();
        assert_eq!(back[0].0, 2);
        assert_eq!(back[0].1.seq, 5);
    }

    #[test]
    fn a_skate_packet_survives_the_wire_and_is_bounded() {
        use crate::transport::protocol::{ClientPacket, ConnectionId, PacketHeader};
        let packet = ClientPacket::Skate {
            header: PacketHeader {
                connection: ConnectionId(9),
                sequence: 4,
                ack: 3,
                epoch: 1,
            },
            claimed_client: 2,
            state: state(1, 0, Vec3::ZERO).to_bytes(),
        };
        let bytes = packet.to_bytes();
        let back = ClientPacket::decode(&mut WireReader::new(&bytes)).unwrap();
        assert_eq!(back, packet);
        let ClientPacket::Skate { state: inner, .. } = back else {
            unreachable!()
        };
        assert!(SkaterState::from_bytes(&inner).unwrap().active);
        let mut long = bytes.clone();
        // tag, header (connection, sequence, ack, epoch), client: the length.
        let at = 1 + 8 + 12 + 4;
        long[at..at + 2].copy_from_slice(&4096u16.to_le_bytes());
        assert!(ClientPacket::decode(&mut WireReader::new(&long)).is_err());
    }

    #[test]
    fn silent_skaters_are_dropped() {
        let mut tracks = SkaterTracks::default();
        tracks.push(2, 0.0, state(1, 0, Vec3::ZERO));
        tracks.prune(1000.0);
        assert_eq!(tracks.tracks.len(), 1);
        tracks.prune(1600.0);
        assert!(tracks.tracks.is_empty());
    }
}
