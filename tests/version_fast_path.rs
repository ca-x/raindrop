use std::process::Command;

#[test]
fn version_exits_before_runtime_initialization() {
    let output = Command::new(env!("CARGO_BIN_EXE_raindrop"))
        .arg("--version")
        .env("RAINDROP_BIND", "not-an-address")
        .output()
        .expect("run raindrop --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("raindrop {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}
