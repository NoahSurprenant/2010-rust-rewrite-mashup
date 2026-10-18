use bevy::prelude::Resource;
use playerstate_iw4::UserCmd;
use sim::{ClientId, Tick};
use std::collections::{HashMap, VecDeque};

use crate::client::predict::CmdSeq;

pub const AUTHORITY_HZ: f64 = 20.0;

pub const AUTHORITY_MS: i32 = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerTime(i32);

impl ServerTime {
    pub const fn from_ms(ms: i32) -> Self {
        Self(ms)
    }

    pub fn from_tick(tick: Tick) -> Self {
        let ms = i32::try_from(tick.0)
            .unwrap_or(0)
            .saturating_mul(AUTHORITY_MS);
        Self(ms)
    }

    pub const fn ms(self) -> i32 {
        self.0
    }
}

pub const MAX_REDUNDANT_CMDS: usize = crate::client::predict::DEFAULT_HISTORY_CAP;

pub const MAX_QUEUED_COMMANDS_PER_PEER: usize = MAX_REDUNDANT_CMDS;

pub const MAX_QUEUED_COMMAND_MS: i32 = 1000;

pub(crate) const COMMAND_TIME_STEP_MS: i32 = 200;

pub const MAX_COMMAND_STALL_MS: i32 = 1000;

pub const MAX_COMMANDS_PER_PEER_PER_FRAME: usize = 32;

/// How long the next command in sequence may wait only because it is dated
/// after this authority's clock before it is taken anyway, one a tick, at the
/// authority's time. A peer's clock runs ahead when this authority lost time
/// (a frame longer than the fixed step catches up): the peer cannot step its
/// clock back, and its commands would otherwise wait until the stall faults it.
pub const FUTURE_HEAD_REBASE_MS: i32 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputBacklogFault {
    QueueCount { queued: usize },

    QueueDuration { queued_ms: i32 },

    NoProgress { stalled_ms: i32 },
}

impl core::fmt::Display for InputBacklogFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::QueueCount { queued } => write!(
                f,
                "InputBacklogExceeded: {queued} unique commands queued (max \
                 {MAX_QUEUED_COMMANDS_PER_PEER})"
            ),
            Self::QueueDuration { queued_ms } => write!(
                f,
                "InputBacklogExceeded: {queued_ms} ms of command time queued (max \
                 {MAX_QUEUED_COMMAND_MS})"
            ),
            Self::NoProgress { stalled_ms } => write!(
                f,
                "InputBacklogExceeded: command queue made no progress for {stalled_ms} ms (max \
                 {MAX_COMMAND_STALL_MS})"
            ),
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityClock {
    pub tick: u32,
    pub time_ms: i32,
}

impl AuthorityClock {
    pub fn advance(&mut self) -> u32 {
        self.time_ms = self.time_ms.wrapping_add(AUTHORITY_MS);
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GatheredCommands {
    pub cmds: Vec<(ClientId, UserCmd)>,

    pub acks: Vec<(ClientId, CmdSeq)>,

    pub proxied: Vec<ClientId>,

    pub samples: Vec<(ClientId, i32, sim::ShotSampleProvenance)>,

    pub backlog_faults: Vec<(ClientId, InputBacklogFault)>,
}

#[derive(Resource, Debug, Default)]
pub struct ClientCommandInbox {
    queued: HashMap<ClientId, VecDeque<(Option<CmdSeq>, UserCmd, sim::ShotSampleProvenance)>>,

    last_consumed: HashMap<ClientId, UserCmd>,

    last_acked_seq: HashMap<ClientId, u32>,

    blocked_head: HashMap<ClientId, (CmdSeq, i32)>,

    rebasing: std::collections::HashSet<ClientId>,
}

impl ClientCommandInbox {
    pub fn push(
        &mut self,
        id: ClientId,
        seq: Option<CmdSeq>,
        cmd: UserCmd,
        sample: Option<sim::ShotSampleProvenance>,
    ) {
        if let Some(s) = seq {
            if self
                .last_acked_seq
                .get(&id)
                .is_some_and(|&last| s.0 <= last)
            {
                return;
            }
            if self
                .queued
                .get(&id)
                .is_some_and(|q| q.iter().any(|(qs, _, _)| *qs == Some(s)))
            {
                return;
            }
        }
        let queue = self.queued.entry(id).or_default();

        if queue.len() > MAX_QUEUED_COMMANDS_PER_PEER {
            return;
        }
        let row = (
            seq,
            cmd,
            sample.unwrap_or(sim::ShotSampleProvenance::NO_CLAIM),
        );
        if let Some(sequence) = seq {
            let at = queue
                .iter()
                .position(|(other, _, _)| other.is_some_and(|other| other > sequence))
                .unwrap_or(queue.len());
            queue.insert(at, row);
        } else {
            queue.push_back(row);
        }
    }

    pub fn clear(&mut self) {
        self.queued.clear();
        self.last_consumed.clear();
        self.last_acked_seq.clear();
        self.blocked_head.clear();
        self.rebasing.clear();
    }

    pub fn retire_client(&mut self, id: ClientId) {
        self.queued.remove(&id);
        self.last_consumed.remove(&id);
        self.last_acked_seq.remove(&id);
        self.blocked_head.remove(&id);
        self.rebasing.remove(&id);
    }

    pub fn pending(&self, id: ClientId) -> usize {
        self.queued.get(&id).map_or(0, VecDeque::len)
    }

    pub fn unique_queue_depth(&self, id: ClientId) -> usize {
        self.pending(id)
    }

    pub fn backlog_fault(&mut self, id: ClientId, time_ms: i32) -> Option<InputBacklogFault> {
        let queue = self.queued.get(&id)?;
        let sequenced = queue.len() - queue.iter().filter(|(seq, _, _)| seq.is_none()).count();
        if sequenced == 0 {
            return None;
        }
        if sequenced > MAX_QUEUED_COMMANDS_PER_PEER {
            return Some(InputBacklogFault::QueueCount { queued: sequenced });
        }
        let mut queued_ms: i32 = 0;
        let mut previous: Option<i32> = self
            .last_consumed
            .get(&id)
            .map(|cmd| cmd.server_time)
            .or_else(|| {
                queue
                    .iter()
                    .find(|(seq, _, _)| seq.is_some())
                    .map(|(_, cmd, _)| time_ms.min(cmd.server_time))
            });
        for (_, cmd, _) in queue.iter().filter(|(seq, _, _)| seq.is_some()) {
            if let Some(previous) = previous {
                queued_ms = queued_ms.saturating_add(
                    cmd.server_time
                        .saturating_sub(previous)
                        .clamp(0, COMMAND_TIME_STEP_MS),
                );
            }
            previous = Some(cmd.server_time);
        }
        if queued_ms > MAX_QUEUED_COMMAND_MS {
            return Some(InputBacklogFault::QueueDuration { queued_ms });
        }
        let head = queue.iter().find_map(|(seq, _, _)| *seq)?;
        let blocked = self.blocked_head.entry(id).or_insert((head, time_ms));
        if blocked.0 != head {
            *blocked = (head, time_ms);
        }
        let stalled_ms = time_ms.saturating_sub(blocked.1);
        if stalled_ms > MAX_COMMAND_STALL_MS {
            return Some(InputBacklogFault::NoProgress { stalled_ms });
        }
        None
    }

    pub fn last_acked_seq(&self, id: ClientId) -> Option<CmdSeq> {
        self.last_acked_seq.get(&id).copied().map(CmdSeq)
    }

    pub fn last_consumed(&self, id: ClientId) -> Option<UserCmd> {
        self.last_consumed.get(&id).copied()
    }

    fn known_clients(&self) -> Vec<ClientId> {
        let mut ids: Vec<ClientId> = self
            .queued
            .keys()
            .chain(self.last_consumed.keys())
            .copied()
            .collect();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        ids
    }

    pub fn take_for_tick(&mut self, time_ms: i32) -> GatheredCommands {
        let mut out = GatheredCommands::default();
        for id in self.known_clients() {
            // A datagram burst can contain more than one frame of runnable work.
            // Consume the bounded frame budget before measuring remaining debt.
            // Count and blocked-head limits still apply before any simulation.
            if let Some(fault) = self.backlog_fault(id, time_ms)
                && !matches!(fault, InputBacklogFault::QueueDuration { .. })
            {
                out.backlog_faults.push((id, fault));
                continue;
            }

            let mut expected = self
                .last_acked_seq
                .get(&id)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            let mut rebase = self.rebasing.contains(&id)
                || self.blocked_head.get(&id).is_some_and(|&(head, since)| {
                    head.0 == expected && time_ms.saturating_sub(since) >= FUTURE_HEAD_REBASE_MS
                });
            let mut rebased = false;
            let take = self
                .queued
                .get(&id)
                .map(|queue| {
                    queue
                        .iter()
                        .take_while(|(seq, cmd, _)| {
                            if let Some(seq) = seq {
                                if seq.0 != expected {
                                    return false;
                                }
                                if cmd.server_time > time_ms {
                                    if !rebase {
                                        return false;
                                    }
                                    rebase = false;
                                    rebased = true;
                                }
                                expected = expected.wrapping_add(1);
                            }
                            true
                        })
                        .take(MAX_COMMANDS_PER_PEER_PER_FRAME)
                        .count()
                })
                .unwrap_or(0);
            let mut drained: Vec<_> = self
                .queued
                .get_mut(&id)
                .map(|queue| queue.drain(..take).collect())
                .unwrap_or_default();
            if drained.iter().all(|(seq, _, _)| seq.is_none()) && drained.len() > 1 {
                drained.drain(..drained.len() - 1);
            }
            drained.sort_by_key(|(seq, _, _)| *seq);
            if drained.is_empty() {
                if let Some(fault) = self.backlog_fault(id, time_ms) {
                    out.backlog_faults.push((id, fault));
                    continue;
                }
                if let Some(&acked) = self.last_acked_seq.get(&id) {
                    out.acks.push((id, CmdSeq(acked)));
                    continue;
                }
                if let Some(mut cmd) = self.last_consumed.get(&id).copied() {
                    cmd.server_time = time_ms;
                    self.last_consumed.insert(id, cmd);
                    out.proxied.push(id);
                    out.cmds.push((id, cmd));
                }
                continue;
            }
            let caught_up = self.queued.get(&id).is_none_or(VecDeque::is_empty);
            if rebased && !caught_up {
                self.rebasing.insert(id);
            } else if caught_up || drained.iter().any(|(seq, _, _)| seq.is_some()) {
                self.rebasing.remove(&id);
            }
            self.blocked_head.remove(&id);
            let mut ack = None;
            for (seq, mut cmd, sample) in drained {
                cmd.server_time = if seq.is_some() {
                    cmd.server_time.min(time_ms)
                } else {
                    time_ms
                };
                self.last_consumed.insert(id, cmd);
                if let Some(seq) = seq {
                    self.last_acked_seq.insert(id, seq.0);
                    ack = Some(seq);
                }

                out.samples.push((id, cmd.server_time, sample));
                out.cmds.push((id, cmd));
            }
            if let Some(seq) = ack {
                out.acks.push((id, seq));
            }
            if let Some(fault) = self.backlog_fault(id, time_ms) {
                out.backlog_faults.push((id, fault));
            }
        }
        out
    }
}

pub const MAX_PENDING_ACTIONS_PER_CLIENT: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEnqueueError {
    QueueFull { pending: usize },

    DuplicateRequestId { request_id: sim::ActionRequestId },
}

impl core::fmt::Display for ActionEnqueueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::QueueFull { pending } => {
                write!(f, "action queue full ({pending} unresolved requests)")
            }
            Self::DuplicateRequestId { request_id } => {
                write!(
                    f,
                    "request_id {request_id} is already pending with another payload"
                )
            }
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct ClientActionInbox {
    pending: Vec<(ClientId, sim::ClientAction)>,
    started: HashMap<(ClientId, sim::ActionRequestId), std::time::Instant>,

    received: Vec<(ClientId, sim::ClientAction)>,
    overflowed: std::collections::HashSet<ClientId>,
}

impl ClientActionInbox {
    pub fn push(
        &mut self,
        id: ClientId,
        action: sim::ClientAction,
    ) -> Result<(), ActionEnqueueError> {
        let request_id = sim::action_request_id(&action);
        for (owner, existing) in &self.pending {
            if *owner != id || sim::action_request_id(existing) != request_id {
                continue;
            }
            if *existing == action {
                return Ok(());
            }
            return Err(ActionEnqueueError::DuplicateRequestId { request_id });
        }
        let held = self
            .pending
            .iter()
            .filter(|(owner, _)| *owner == id)
            .count();
        if held >= MAX_PENDING_ACTIONS_PER_CLIENT {
            return Err(ActionEnqueueError::QueueFull { pending: held });
        }
        self.pending.push((id, action));
        Ok(())
    }

    pub fn mark_dispatched(&mut self, id: ClientId, request_id: sim::ActionRequestId) {
        if self
            .pending
            .iter()
            .any(|(owner, action)| *owner == id && sim::action_request_id(action) == request_id)
        {
            self.started
                .entry((id, request_id))
                .or_insert_with(std::time::Instant::now);
        }
    }

    pub fn push_from_peer(&mut self, id: ClientId, action: sim::ClientAction) {
        if self.received.contains(&(id, action)) {
            return;
        }
        if self
            .received
            .iter()
            .filter(|(owner, _)| *owner == id)
            .count()
            >= MAX_PENDING_ACTIONS_PER_CLIENT
        {
            self.overflowed.insert(id);
            return;
        }
        self.received.push((id, action));
    }

    pub fn take_overflowed(&mut self) -> impl Iterator<Item = ClientId> + '_ {
        self.overflowed.drain()
    }

    pub fn timed_out(&self) -> bool {
        self.started
            .values()
            .any(|at| at.elapsed() > std::time::Duration::from_secs(8))
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn pending(&self) -> &[(ClientId, sim::ClientAction)] {
        &self.pending
    }

    pub fn pending_for(&self, id: ClientId) -> impl Iterator<Item = sim::ClientAction> + '_ {
        self.pending
            .iter()
            .filter(move |(owner, _)| *owner == id)
            .map(|(_, action)| *action)
    }

    pub fn retire(&mut self, id: ClientId, request_id: sim::ActionRequestId) -> bool {
        self.started.remove(&(id, request_id));
        let before = self.pending.len();
        self.pending
            .retain(|(owner, action)| *owner != id || sim::action_request_id(action) != request_id);
        self.pending.len() != before
    }

    pub fn retire_client(&mut self, id: ClientId) {
        self.started.retain(|(owner, _), _| *owner != id);
        self.pending.retain(|(owner, _)| *owner != id);
        self.received.retain(|(owner, _)| *owner != id);
        self.overflowed.remove(&id);
    }

    pub fn clear(&mut self) {
        self.started.clear();
        self.pending.clear();
        self.received.clear();
        self.overflowed.clear();
    }

    /// Every action to take this tick, each with whether a remote peer sent
    /// it (rather than this game's own console, menus or bots).
    pub fn gather(&mut self) -> Vec<(ClientId, sim::ClientAction, bool)> {
        let mut out: Vec<_> = self
            .pending
            .iter()
            .map(|&(id, action)| (id, action, false))
            .collect();
        for (id, action, _) in &out {
            self.mark_dispatched(*id, sim::action_request_id(action));
        }
        out.extend(
            self.received
                .drain(..)
                .map(|(id, action)| (id, action, true)),
        );
        out
    }
}

pub fn run_fixed_authority_stream(
    world: &mut sim::SimWorld,
    ticks: u32,
    mut sample: impl FnMut(u32, i32) -> Vec<(ClientId, UserCmd)>,
) -> Vec<sim::Snapshot> {
    let mut clock = AuthorityClock::default();
    let mut out = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        let tick = clock.advance();
        let cmds = sample(tick, clock.time_ms);
        out.push(sim::step(
            world,
            sim::Tick(tick),
            &sim::TickInput::from_cmds(cmds),
            AUTHORITY_MS,
            sim::StepReason::AuthorityFrame,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: ClientId = ClientId(1);

    fn cmd(server_time: i32) -> UserCmd {
        UserCmd {
            server_time,
            ..UserCmd::default()
        }
    }

    /// One authority tick: the commands it took for `PEER` and any fault.
    fn tick(inbox: &mut ClientCommandInbox, time_ms: i32) -> (Vec<i32>, Vec<InputBacklogFault>) {
        let out = inbox.take_for_tick(time_ms);
        let taken = out
            .cmds
            .iter()
            .filter(|(id, _)| *id == PEER)
            .map(|(_, cmd)| cmd.server_time)
            .collect();
        let faults = out.backlog_faults.iter().map(|(_, f)| *f).collect();
        (taken, faults)
    }

    #[test]
    fn a_peer_whose_clock_runs_ahead_is_taken_a_command_a_tick_not_retired() {
        let mut inbox = ClientCommandInbox::default();
        let ahead = 1500;
        let mut taken_per_tick = Vec::new();
        for n in 1..=120u32 {
            let time_ms = n as i32 * AUTHORITY_MS;
            inbox.push(PEER, Some(CmdSeq(n)), cmd(time_ms + ahead), None);
            let (taken, faults) = tick(&mut inbox, time_ms);
            assert!(faults.is_empty(), "tick {n}: {faults:?}");
            assert!(taken.iter().all(|&t| t <= time_ms), "tick {n}: {taken:?}");
            taken_per_tick.push(taken.len());
        }
        let waited = taken_per_tick.iter().take_while(|&&n| n == 0).count() as i32;
        assert!(waited * AUTHORITY_MS <= FUTURE_HEAD_REBASE_MS + AUTHORITY_MS);
        assert!(taken_per_tick[waited as usize..].iter().all(|&n| n == 1));
        assert!(inbox.pending(PEER) <= (waited + 1) as usize);
        assert_eq!(
            inbox.last_acked_seq(PEER),
            Some(CmdSeq(120 - waited as u32))
        );
    }

    #[test]
    fn a_command_dated_a_tick_ahead_waits_for_its_tick() {
        let mut inbox = ClientCommandInbox::default();
        inbox.push(PEER, Some(CmdSeq(1)), cmd(100), None);
        assert_eq!(tick(&mut inbox, 100).0, vec![100]);
        inbox.push(PEER, Some(CmdSeq(2)), cmd(150), None);
        assert!(tick(&mut inbox, 100).0.iter().all(|&t| t != 150));
        assert_eq!(tick(&mut inbox, 150).0, vec![150]);
    }

    #[test]
    fn once_the_peer_s_clock_is_back_its_commands_wait_for_their_tick_again() {
        let mut inbox = ClientCommandInbox::default();
        let mut seq = 0;
        let mut time_ms = 0;
        for _ in 0..20 {
            time_ms += AUTHORITY_MS;
            seq += 1;
            inbox.push(PEER, Some(CmdSeq(seq)), cmd(time_ms + 1000), None);
            tick(&mut inbox, time_ms);
        }
        // The peer's clock holds while the authority's catches up with it.
        for _ in 0..40 {
            time_ms += AUTHORITY_MS;
            assert!(tick(&mut inbox, time_ms).1.is_empty());
        }
        assert_eq!(inbox.pending(PEER), 0);
        time_ms += AUTHORITY_MS;
        seq = inbox.last_acked_seq(PEER).map_or(0, |s| s.0) + 1;
        inbox.push(PEER, Some(CmdSeq(seq)), cmd(time_ms + AUTHORITY_MS), None);
        assert!(
            tick(&mut inbox, time_ms)
                .0
                .iter()
                .all(|&t| t != time_ms + AUTHORITY_MS)
        );
        assert_eq!(
            tick(&mut inbox, time_ms + AUTHORITY_MS).0,
            vec![time_ms + AUTHORITY_MS]
        );
    }

    #[test]
    fn a_command_that_never_comes_still_retires_the_peer() {
        let mut inbox = ClientCommandInbox::default();
        let mut faults = Vec::new();
        for n in 2..=40u32 {
            let time_ms = n as i32 * AUTHORITY_MS;
            inbox.push(PEER, Some(CmdSeq(n)), cmd(time_ms), None);
            faults.extend(tick(&mut inbox, time_ms).1);
            if !faults.is_empty() {
                break;
            }
        }
        assert!(!faults.is_empty());
    }
}
