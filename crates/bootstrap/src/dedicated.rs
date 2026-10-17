//! The dedicated server's process lifetime. It runs one match for as long
//! as its room lives: SIGTERM or SIGINT ends it cleanly, and a room the
//! master lost or a match that ended ends it with an error, for a
//! supervisor to start it again. Every exit goes through `AppExit` from
//! `First`, so the frame's `Update` still sees it (the Minecraft world saves
//! there) before `Last` ends the process.
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use frame::{MatchInstalled, MatchTornDown};
use net::MasterBridge;

pub const DEFAULT_ROOM_NAME: &str = "Dedicated server";

static STOP_SIGNAL: AtomicBool = AtomicBool::new(false);

/// Frames between a torn-down match and the exit: the world it held saves
/// on the teardown first.
const TEARDOWN_GRACE_FRAMES: u32 = 30;

pub(crate) fn register(app: &mut App) {
    install_stop_signal();
    app.add_systems(First, end_dedicated_server)
        .add_systems(Update, announce_players);
}

/// Who is in the match, whenever that changes: `dedicated-players: count=N
/// clients=1,2 names=a,b` on stdout.
fn announce_players(
    authority: Option<Res<net::AuthorityWorld>>,
    mut last: Local<Option<Vec<(u32, String)>>>,
) {
    let Some(authority) = authority else {
        return;
    };
    let players: Vec<(u32, String)> = authority
        .0
        .clients_scoreboard()
        .into_iter()
        .map(|(id, _)| (id.0, net::client_name_string(&authority.0, id)))
        .collect();
    if last.as_ref() == Some(&players) {
        return;
    }
    let ids: Vec<String> = players.iter().map(|(id, _)| id.to_string()).collect();
    let names: Vec<&str> = players.iter().map(|(_, name)| name.as_str()).collect();
    let line = format!(
        "dedicated-players: count={} clients={} names={}",
        players.len(),
        ids.join(","),
        names.join(",")
    );
    println!("{line}");
    *last = Some(players);
}

#[cfg(target_os = "linux")]
fn install_stop_signal() {
    extern "C" fn on_stop(_: libc::c_int) {
        STOP_SIGNAL.store(true, Ordering::SeqCst);
    }
    let handler = on_stop as extern "C" fn(libc::c_int);
    unsafe {
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }
}

#[cfg(not(target_os = "linux"))]
fn install_stop_signal() {}

#[derive(Default)]
struct Lifetime {
    installed: bool,
    torn_down: Option<u32>,
    exiting: bool,
}

fn end_dedicated_server(
    mut installed: MessageReader<MatchInstalled>,
    mut torn_down: MessageReader<MatchTornDown>,
    bridge: Option<Res<MasterBridge>>,
    mut exit: MessageWriter<AppExit>,
    mut lifetime: Local<Lifetime>,
) {
    if installed.read().next().is_some() {
        lifetime.installed = true;
    }
    if torn_down.read().next().is_some() && lifetime.installed && lifetime.torn_down.is_none() {
        diag::warn!(Launch, "dedicated server: the match ended");
        lifetime.torn_down = Some(TEARDOWN_GRACE_FRAMES);
    }
    if lifetime.exiting {
        return;
    }
    if STOP_SIGNAL.load(Ordering::SeqCst) {
        diag::info!(Launch, "dedicated server: stopping on SIGTERM or SIGINT");
        lifetime.exiting = true;
        exit.write(AppExit::Success);
        return;
    }
    if let Some(bridge) = bridge
        && bridge.state().is_terminal()
    {
        diag::error!(
            Launch,
            "dedicated server: the master closed its room: {:?}",
            bridge.state()
        );
        lifetime.exiting = true;
        exit.write(AppExit::error());
        return;
    }
    if let Some(frames) = lifetime.torn_down.as_mut() {
        *frames = frames.saturating_sub(1);
        if *frames == 0 {
            lifetime.exiting = true;
            exit.write(AppExit::error());
        }
    }
}
