//! `admins`: the players a host lets run debug commands (`net::DebugAdmins`),
//! shown or set from the host's own console, and the line the approved
//! tests read (`debug-admins:`).
use bevy::prelude::*;

use crate::{ConsoleCommand, ConsoleLine, ConsoleSettings, ConsoleState};

pub(crate) const USAGE: &str = "admins [set <name,name,...|-> [label]] - players allowed debug commands on this host (IW4L_ADMINS)";

pub(crate) fn route_admins_commands(
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut line: ResMut<ConsoleLine>,
    admins: Option<ResMut<net::DebugAdmins>>,
) {
    let capacity = settings.log_capacity;
    let mut admins = admins;
    for cmd in events.read() {
        if cmd.name != "admins" {
            continue;
        }
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let text = match (args.as_slice(), admins.as_mut()) {
            (_, None) => "admins: not a host".to_owned(),
            ([], Some(admins)) => admins_line(admins, None),
            (["set", list, label @ ..], Some(admins)) if label.len() <= 1 => {
                **admins = net::DebugAdmins::parse(if *list == "-" { "" } else { list });
                diag::info!(Net, "admins: {}", admins.describe());
                admins_line(admins, label.first().copied())
            }
            _ => format!("usage: {USAGE}"),
        };
        println!("{text}");
        diag::info!(Console, "{text}");
        line.0 = text.clone();
        console.echo(text, capacity);
    }
}

fn admins_line(admins: &net::DebugAdmins, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("debug-admins: label={label} names={}", admins.describe()),
        None => format!("debug-admins: names={}", admins.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_admins_or_none() {
        let admins = net::DebugAdmins::parse("noah,client");
        assert_eq!(
            admins_line(&admins, Some("s_admins")),
            "debug-admins: label=s_admins names=noah,client"
        );
        assert_eq!(
            admins_line(&net::DebugAdmins::default(), None),
            "debug-admins: names=-"
        );
    }
}
