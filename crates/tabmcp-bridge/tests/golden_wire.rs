//! Golden wire-format test: the simulator's canned song serialized to JSON
//! must match the checked-in fixture byte-for-byte. Any serde change that
//! alters the wire format (renames, defaults, effect shapes) fails here
//! BEFORE it breaks the Java plugin.
//!
//! To intentionally update the fixture after a deliberate protocol change:
//!   UPDATE_GOLDEN=1 cargo test -p tabmcp-bridge --test golden_wire

use tabmcp_bridge::sim;

#[test]
fn wire_format_matches_golden_fixture() {
    let payload = serde_json::json!({
        "song": sim::demo_song(),
        "measures": sim::demo_measures(1, 4),
    });
    let current = serde_json::to_string_pretty(&payload).expect("serializes");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/demo_wire.json");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, &current).expect("write golden");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("golden fixture missing — run with UPDATE_GOLDEN=1 to create it");
    assert_eq!(
        current, golden,
        "wire format changed! If intentional, bump the protocol notes and \
         regenerate with UPDATE_GOLDEN=1; if not, you just avoided breaking \
         the Java plugin."
    );
}
