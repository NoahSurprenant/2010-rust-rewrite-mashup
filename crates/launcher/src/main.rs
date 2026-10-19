use std::path::PathBuf;

use asset_transport::{ensure_artifacts_dir, games_root_from_env};

#[cfg(windows)]
mod first_run;

#[global_allocator]
static PROCESS_ALLOCATOR: diag::ProcessCountingAllocator = diag::ProcessCountingAllocator;

fn main() {
    let mut cli = std::env::args().skip(1);
    if cli.next().as_deref() == Some("dedicated-console") {
        dedicated_console(cli.collect::<Vec<_>>().join(" "));
    }
    bootstrap::bench::arm();
    prepare_process_root().unwrap_or_else(|e| {
        diag::exit_launch_error(&e);
    });
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    {
        hand_to_launcher(&args);
        // Also for a shortcut that names a map, so it works on first launch.
        first_run::prepare().unwrap_or_else(|e| first_run::fail(&e));
        if args.is_empty() {
            args.push("menu".into());
        }
    }
    let artifacts = ensure_artifacts_dir().unwrap_or_else(|e| diag::exit_launch_error(&e));
    announce_log(diag::init_log(&artifacts));
    let (mode, acceptance) =
        bootstrap::parse_cli(args.into_iter()).unwrap_or_else(|e| diag::exit_launch_error(&e));
    let games = games_root_from_env().unwrap_or_else(|e| diag::exit_launch_error(&e));
    bootstrap::launch(games, artifacts, mode, acceptance);
}

/// A release build that updates itself (this fork's, built with
/// `IW4L_DEFAULT_UPDATE_URL`) starts through `iw4launcher.exe` beside it,
/// which fetches a newer build first and starts this one again with the same
/// arguments (`IW4L_LAUNCHER=1`). `IW4L_UPDATE=0`, or no launcher beside
/// it, starts the game here.
#[cfg(windows)]
fn hand_to_launcher(args: &[String]) {
    if option_env!("IW4L_DEFAULT_UPDATE_URL").is_none_or(|url| url.trim().is_empty())
        || std::env::var_os("IW4L_LAUNCHER").is_some()
        || std::env::var("IW4L_UPDATE").is_ok_and(|value| {
            let value = value.trim();
            value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false")
        })
    {
        return;
    }
    let Ok(dir) = std::env::current_dir() else {
        return;
    };
    let launcher = dir.join("iw4launcher.exe");
    if !launcher.is_file() {
        return;
    }
    match std::process::Command::new(&launcher)
        .args(args)
        .current_dir(&dir)
        .spawn()
    {
        Ok(_) => std::process::exit(0),
        Err(error) => eprintln!(
            "[iw4l] could not start {}: {error}; starting here",
            launcher.display()
        ),
    }
}

fn prepare_process_root() -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe =
            std::env::current_exe().map_err(|error| format!("cannot locate iw4l.exe: {error}"))?;
        let root = exe
            .parent()
            .ok_or_else(|| format!("iw4l.exe has no parent directory: {}", exe.display()))?;
        std::env::set_current_dir(root).map_err(|error| {
            format!(
                "cannot enter launcher directory {}: {error}",
                root.display()
            )
        })?;
    }
    Ok(())
}

fn announce_log(path: PathBuf) {
    diag::announce_log_stdout(&path, diag::latest_log_path().as_deref());
    diag::info!(Launch, "log: {}", path.display());
}

/// `iw4l dedicated-console "<command>"`: one command to the console of the
/// dedicated server on this machine (its `IW4L_CONSOLE_SOCKET`), its answer
/// printed; exits with 1 when no server answers.
fn dedicated_console(command: String) -> ! {
    let path = console::control::socket_path();
    match console::control::ask(&path, &command) {
        Ok(answer) => {
            print!("{answer}");
            std::process::exit(0)
        }
        Err(e) => {
            eprintln!(
                "dedicated-console: {}: {e} (a dedicated server started with {}={} answers there)",
                path.display(),
                console::control::SOCKET_ENV,
                path.display()
            );
            std::process::exit(1)
        }
    }
}
