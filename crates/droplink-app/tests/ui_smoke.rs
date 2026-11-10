use std::time::{Duration, Instant};

#[test]
fn smoke_starts_fast() {
    let home = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_droplink"))
        .arg("--smoke")
        .env("HOME", home.path())
        .output()
        .expect("failed to run droplink --smoke");
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "smoke exited with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("smoke ok"), "unexpected output: {stdout}");

    // The app reports its own startup time (config + engine + mDNS), which is the budget we own.
    let startup_ms: u128 = stdout
        .lines()
        .find_map(|line| line.strip_prefix("startup_ms="))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(u128::MAX);
    assert!(
        startup_ms < 250,
        "internal startup took {startup_ms} ms, above the 250 ms budget"
    );

    // Process spawn itself is bounded separately (CI machines are noisy).
    assert!(
        elapsed < Duration::from_secs(5),
        "process took {elapsed:?} to complete"
    );
}
