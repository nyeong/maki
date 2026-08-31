use std::process::Command;

use super::BIN;

#[test]
fn maki_reports_text_and_machine_readable_versions() {
    let text = Command::new(BIN).arg("--version").output().unwrap();
    assert!(text.status.success());
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        format!("maki {}\n", env!("CARGO_PKG_VERSION"))
    );

    let json = Command::new(BIN)
        .args(["--version", "--json"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["name"], "maki");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    let source_revision = option_env!("MAKI_SOURCE_REVISION").filter(|revision| {
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    assert_eq!(
        value["source_revision"],
        source_revision
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null)
    );
}
