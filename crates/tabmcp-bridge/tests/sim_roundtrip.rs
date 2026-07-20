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
