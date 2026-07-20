//! `tabmcp` — TabMCP command-line entry point.
//!
//! Subcommands:
//!   serve       MCP server over stdio (arrives in Phase 3)
//!   doctor      connect to the TuxGuitar bridge and print a status report
//!   bridge-sim  run a simulated bridge (for development and CI)

use std::path::PathBuf;
use std::process::ExitCode;

use tabmcp_bridge::{default_discovery_path, sim, BridgeClient};
use tabmcp_theory::note_name;

mod audio;
mod render;
mod serve;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut discovery_path = default_discovery_path();
    let mut positional = Vec::new();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--discovery-file" => match iter.next() {
                Some(path) => discovery_path = PathBuf::from(path),
                None => return usage("--discovery-file requires a path"),
            },
            "--help" | "-h" => return usage(""),
            _ => positional.push(arg.clone()),
        }
    }

    match positional.first().map(String::as_str) {
        Some("doctor") => doctor(&discovery_path),
        Some("spike-test") => spike_test(&discovery_path),
        Some("bridge-sim") => bridge_sim(&discovery_path),
        Some("serve") => match serve::run(&discovery_path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("tabmcp serve: {message}");
                ExitCode::FAILURE
            }
        },
        Some(other) => usage(&format!("unknown subcommand: {other}")),
        None => usage(""),
    }
}

fn usage(error: &str) -> ExitCode {
    if !error.is_empty() {
        eprintln!("error: {error}\n");
    }
    eprintln!(
        "usage: tabmcp [--discovery-file <path>] <subcommand>\n\n\
         subcommands:\n  \
         doctor       connect to the TuxGuitar bridge and print a status report\n  \
         spike-test   run the Milestone-1 undoable edit + undo + redo over the bridge\n  \
         bridge-sim   run a simulated bridge until interrupted\n  \
         serve        MCP server over stdio (Phase 3)"
    );
    if error.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

fn doctor(discovery_path: &std::path::Path) -> ExitCode {
    println!("tabmcp doctor");
    println!("  discovery file : {}", discovery_path.display());

    let mut client = match BridgeClient::connect(discovery_path) {
        Ok(client) => client,
        Err(e) => {
            println!("  bridge         : NOT CONNECTED");
            println!("  reason         : {e}");
            return ExitCode::FAILURE;
        }
    };

    let hello = client.hello_info().clone();
    let discovery = client.discovery_info().clone();
    println!(
        "  bridge         : connected (127.0.0.1:{})",
        discovery.port
    );
    println!("  tuxguitar      : {}", hello.server_info.tuxguitar_version);
    println!("  plugin         : {}", hello.server_info.plugin_version);
    println!("  protocol       : v{}", hello.protocol_version);
    println!("  capabilities   : {}", hello.capabilities.join(", "));

    match client.ping() {
        Ok(ping) => {
            println!("  document open  : {}", ping.document_open);
            println!("  revision       : {}", ping.revision);
        }
        Err(e) => {
            println!("  ping failed    : {e}");
            return ExitCode::FAILURE;
        }
    }

    match client.read_song() {
        Ok(song) => {
            let title = if song.metadata.title.is_empty() {
                "(untitled)"
            } else {
                &song.metadata.title
            };
            println!("\n  song           : {title}");
            println!("  measures       : {}", song.headers.len());
            if let Some(header) = song.headers.first() {
                println!(
                    "  starts as      : {}/{} at {} bpm",
                    header.time_signature.numerator,
                    header.time_signature.denominator,
                    header.tempo_bpm
                );
            }
            println!("  tracks         : {}", song.tracks.len());
            for track in &song.tracks {
                let tuning: Vec<String> = track
                    .strings
                    .iter()
                    .map(|s| note_name(s.open_pitch))
                    .collect();
                let kind = if track.is_percussion {
                    " [percussion]"
                } else {
                    ""
                };
                println!(
                    "    {}. {}{} — {} strings ({})",
                    track.number,
                    track.name,
                    kind,
                    track.strings.len(),
                    if track.is_percussion {
                        "-".into()
                    } else {
                        tuning.join(" ")
                    },
                );
            }
        }
        Err(e) => {
            println!("  read_song      : FAILED — {e}");
            return ExitCode::FAILURE;
        }
    }

    println!("\n  all checks passed");
    ExitCode::SUCCESS
}

fn spike_test(discovery_path: &std::path::Path) -> ExitCode {
    let mut client = match BridgeClient::connect(discovery_path) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("cannot connect: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("spike-test: hard-coded undoable edit through the real bridge\n");

    let edit = match client.spike_edit() {
        Ok(edit) => edit,
        Err(e) => {
            eprintln!("spike_edit failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "  edit applied  : track {}, measure {} — {} (revision {})",
        edit.track, edit.measure, edit.description, edit.new_revision
    );

    match client.undo() {
        Ok(undo) if undo.performed => {
            println!(
                "  undo          : reverted (revision {})",
                undo.new_revision
            )
        }
        Ok(_) => {
            eprintln!("  undo          : NOT performed — undo stack integration failed");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("  undo failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    match client.redo() {
        Ok(redo) if redo.performed => {
            println!(
                "  redo          : re-applied (revision {})",
                redo.new_revision
            )
        }
        Ok(_) => {
            eprintln!("  redo          : NOT performed — redo integration failed");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("  redo failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    println!(
        "\n  the edit is now applied again — check the TuxGuitar window:\n  \
         measure 1 should show fret 5 (or 7) on string 6, and Cmd+Z should revert it."
    );
    ExitCode::SUCCESS
}

fn bridge_sim(discovery_path: &std::path::Path) -> ExitCode {
    match sim::start(discovery_path) {
        Ok(handle) => {
            println!(
                "bridge simulator listening on 127.0.0.1:{} (discovery: {})",
                handle.port,
                handle.discovery_path.display()
            );
            println!("press Ctrl+C to stop");
            loop {
                std::thread::park();
            }
        }
        Err(e) => {
            eprintln!("failed to start simulator: {e}");
            ExitCode::FAILURE
        }
    }
}
