//! End-to-end test of the bridge client against the simulator: the same
//! sequence `tabmcp doctor` runs against real TuxGuitar.

use tabmcp_bridge::{sim, BridgeClient, BridgeError};

fn temp_discovery_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("tabmcp-test-{}-{}", name, std::process::id()));
    path.push("bridge.json");
    path
}

#[test]
fn full_roundtrip_against_simulator() {
    let discovery_path = temp_discovery_path("roundtrip");
    let handle = sim::start(&discovery_path).expect("simulator starts");

    let mut client = BridgeClient::connect(&discovery_path).expect("client connects");
    assert_eq!(
        client.hello_info().server_info.tuxguitar_version,
        "simulator"
    );
    assert!(client
        .hello_info()
        .capabilities
        .contains(&"read".to_string()));

    let ping = client.ping().expect("ping");
    assert!(ping.document_open);
    let base_revision = ping.revision;

    let song = client.read_song().expect("read_song");
    assert_eq!(song.tracks.len(), 2);
    assert_eq!(song.tracks[0].strings.len(), 6);
    assert_eq!(song.tracks[0].strings[5].open_pitch, 40); // low E
    assert_eq!(song.headers.len(), 4);

    let edit = client.spike_edit().expect("spike_edit");
    assert!(edit.new_revision > base_revision, "edit must bump revision");

    let undo = client.undo().expect("undo");
    assert!(undo.performed, "undo after an edit must succeed");

    let redo = client.redo().expect("redo");
    assert!(redo.performed, "redo after undo must succeed");

    handle.stop();
}

#[test]
fn write_path_applies_and_rejects_stale_revisions() {
    let discovery_path = temp_discovery_path("write");
    let handle = sim::start(&discovery_path).expect("simulator starts");
    let mut client = BridgeClient::connect(&discovery_path).expect("client connects");

    let before = client.read_measures(1, 3, 3).expect("read measure 3");
    assert!(
        before.measures[0].beats[0].voices[0].is_rest,
        "measure 3 starts as a rest"
    );
    let revision = before.revision;

    // Write one E5 quarter note into measure 3.
    let mut measures = sim::demo_measures(3, 3);
    measures[0].beats = vec![tabmcp_model::Beat {
        start_tick: measures[0].start_tick,
        voices: vec![tabmcp_model::Voice {
            index: 0,
            duration: tabmcp_model::Duration {
                value: 4,
                dotted: false,
                double_dotted: false,
                tuplet: tabmcp_model::Tuplet {
                    enters: 1,
                    times: 1,
                },
            },
            is_rest: false,
            notes: vec![tabmcp_model::Note {
                string: 6,
                fret: 12,
                velocity: 95,
                tied: false,
                effects: tabmcp_model::NoteEffects::default(),
            }],
        }],
    }];

    let applied = client
        .apply_replace_measures(1, 3, &measures, revision)
        .expect("apply succeeds at current revision");
    assert_eq!(applied.notes_after, 1);
    assert!(applied.new_revision > revision);

    let after = client.read_measures(1, 3, 3).expect("read back");
    assert_eq!(after.measures[0].beats[0].voices[0].notes[0].fret, 12);

    // Applying against the OLD revision must be rejected.
    match client.apply_replace_measures(1, 3, &measures, revision) {
        Err(BridgeError::Rejected { code, .. }) => assert_eq!(code, "E_STALE_REVISION"),
        other => panic!(
            "expected stale rejection, got: {:?}",
            other.map(|r| r.new_revision)
        ),
    }

    handle.stop();
}

#[test]
fn wrong_token_is_rejected() {
    let discovery_path = temp_discovery_path("badtoken");
    let handle = sim::start(&discovery_path).expect("simulator starts");

    // Corrupt the token on disk, then try to connect.
    let text = std::fs::read_to_string(&discovery_path).unwrap();
    std::fs::write(
        &discovery_path,
        text.replace(
            text.split("\"token\": \"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap(),
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    )
    .unwrap();

    match BridgeClient::connect(&discovery_path) {
        Err(BridgeError::Rejected { code, .. }) => assert_eq!(code, "E_NOT_AUTHENTICATED"),
        Err(other) => panic!("expected auth rejection, got: {other:?}"),
        Ok(_) => panic!("expected auth rejection, but connect succeeded"),
    }
    handle.stop();
}

#[test]
fn missing_discovery_file_reports_not_running() {
    let discovery_path = temp_discovery_path("missing");
    match BridgeClient::connect(&discovery_path) {
        Err(BridgeError::NotRunning(path)) => assert_eq!(path, discovery_path),
        Err(other) => panic!("expected NotRunning, got: {other:?}"),
        Ok(_) => panic!("expected NotRunning, but connect succeeded"),
    }
}
