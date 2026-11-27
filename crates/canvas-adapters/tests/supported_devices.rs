//! Issue #580 (R10, R19): the Help checklist's device list is the adapters' own list. A new
//! adapter that forgets to publish its `SUPPORTED` constant fails this test with its name, so the
//! editor cannot drift into a name or an id the adapters never drive.

use canvas_adapters::{attack_shark_x6, chione, eightbitdo_ultimate2, royuan};
use std::path::PathBuf;

type AdapterList = (&'static str, &'static [(&'static str, u16, u16)]);

fn adapters() -> Vec<AdapterList> {
    vec![
        ("royuan", royuan::SUPPORTED),
        ("chione", chione::SUPPORTED),
        ("attack-shark-x6", attack_shark_x6::SUPPORTED),
        ("eightbitdo-ultimate-2", eightbitdo_ultimate2::SUPPORTED),
    ]
}

#[test]
fn the_editor_list_is_the_adapters_list() {
    let mut expected = Vec::new();
    for (adapter, list) in adapters() {
        assert!(!list.is_empty(), "{adapter} publishes an empty SUPPORTED list");
        for (name, vid, pid) in list {
            expected.push((name.to_string(), format!("{vid:04X}"), format!("{pid:04X}")));
        }
    }
    expected.sort();

    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/web/src/lib/supportedDevices.ts");
    let source = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", file.display()));
    let mut editor = Vec::new();
    for line in source.lines() {
        let line = line.trim().trim_end_matches(',');
        let Some(rest) = line.strip_prefix("{ name: \"") else { continue };
        let mut parts = rest.split("\", ");
        let name = parts.next().expect("a name").to_string();
        let vid_field = parts.next().expect("a vid");
        let pid_field = parts.next().expect("a pid");
        let vid = vid_field.strip_prefix("vid: \"").expect("a vid value").to_string();
        let pid = pid_field
            .strip_prefix("pid: \"")
            .expect("a pid value")
            .trim_end_matches("\" }")
            .to_string();
        editor.push((name, vid, pid));
    }
    editor.sort();

    assert!(!editor.is_empty(), "the editor's supported list could not be read");
    assert_eq!(editor, expected, "the editor's device list matches the adapters' constants");
}
