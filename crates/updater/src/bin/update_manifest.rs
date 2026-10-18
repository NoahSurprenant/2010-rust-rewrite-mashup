//! `iw4l-update-manifest`: writes the manifest a GitHub release publishes
//! beside its files, for `iw4launcher` to update from, and checks one.
//!
//! ```text
//! iw4l-update-manifest write --git <sha> --build <n> --dir <folder> --out <manifest.json>
//! iw4l-update-manifest check --dir <folder> <manifest.json>
//! ```
//!
//! `<folder>` holds `iw4l.exe`, its zstd `iw4l.exe.zst` (what is downloaded)
//! and `iw4launcher.exe`. `check` validates the manifest as the launcher
//! does and that every file it names in `<folder>` matches its hash and size.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("iw4l-update-manifest: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut it = args.into_iter();
    let command = it.next().ok_or("write or check?")?;
    let (mut git, mut build, mut dir, mut out, mut manifest) = (None, None, None, None, None);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--git" => git = Some(value()?),
            "--build" => {
                build = Some(
                    value()?
                        .parse::<u64>()
                        .map_err(|e| format!("--build: {e}"))?,
                )
            }
            "--dir" => dir = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            _ if !arg.starts_with("--") => manifest = Some(PathBuf::from(arg)),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let dir = dir.ok_or("--dir is required")?;
    match command.as_str() {
        "write" => {
            let git = git.ok_or("--git is required")?;
            let build = build.ok_or("--build is required")?;
            let out = out.ok_or("--out is required")?;
            let manifest = updater::release::write(&dir, &git, build, &out)?;
            updater::release::check(&dir, &out)?;
            println!(
                "{}: build {build}, git {git}, {} files",
                out.display(),
                manifest.files.len()
            );
            Ok(())
        }
        "check" => {
            let manifest = manifest.ok_or("which manifest?")?;
            updater::release::check(&dir, &manifest)?;
            println!("{}: ok", manifest.display());
            Ok(())
        }
        other => Err(format!("unknown command {other}")),
    }
}
