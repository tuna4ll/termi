//! The binary's command line, run for real.

use std::process::Command;

fn termi(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_termi"))
        .args(arguments)
        .env(
            "TERMI_CONFIG_DIR",
            std::env::temp_dir().join("termi-cli-config"),
        )
        .env(
            "TERMI_DATA_DIR",
            std::env::temp_dir().join("termi-cli-data"),
        )
        .output()
        .expect("run termi")
}

#[test]
fn version_and_help() {
    let version = termi(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("termi {}", env!("CARGO_PKG_VERSION"))
    );
    let help = termi(&["-h"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("--readonly"));
}

#[test]
fn an_unknown_option_fails_with_the_usage() {
    let output = termi(&["--frobnicate"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unknown option --frobnicate"), "{error}");
    assert!(error.contains("USAGE"), "{error}");
}
