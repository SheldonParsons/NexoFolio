#![cfg(unix)]
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

#[tokio::test]
async fn api_and_worker_exit_cleanly_on_sigterm() {
    for (binary, expected) in [
        (env!("CARGO_BIN_EXE_nexofolio-api"), "api_started"),
        (
            env!("CARGO_BIN_EXE_nexofolio-worker"),
            "worker_started_no_job_source_configured",
        ),
    ] {
        let mut child = Command::new(binary)
            .env_clear()
            .env("DATABASE_URL", "postgres://unused@127.0.0.1:1/unused")
            .env("NEXOFOLIO_BIND_ADDR", "127.0.0.1:0")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let startup = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            startup.contains(expected),
            "unexpected startup output: {startup}"
        );
        let pid = child.id().unwrap();
        assert!(
            Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .await
                .unwrap()
                .success()
        );
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "process exited unsuccessfully: {status}");
    }
}

#[tokio::test]
async fn administration_validates_configuration_without_database_or_secrets_in_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_nexofolio-admin"))
        .arg("check-config")
        .env_clear()
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("DATABASE_URL"));
    let output = Command::new(env!("CARGO_BIN_EXE_nexofolio-admin"))
        .arg("check-config")
        .env_clear()
        .env("DATABASE_URL", "postgres://unused@127.0.0.1:1/unused")
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("connectivity not checked"));
    let output = Command::new(env!("CARGO_BIN_EXE_nexofolio-admin"))
        .arg("check-config")
        .env_clear()
        .env("DATABASE_URL", "invalid://do-not-print-this-password")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("do-not-print-this-password"));
}
