use std::path::PathBuf;
use std::process::{Command, ExitCode};

const SIZE_BUDGET: u64 = 22 * 1024 * 1024;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("perf") => perf(),
        Some("bundle") => run("cargo", &["bundle", "--release", "-p", "droplink-app"]),
        Some("bundle-msi") => run("cargo", &["wix", "-p", "droplink-app"]),
        Some("fmt") => run("cargo", &["fmt", "--all"]),
        Some("lint") => run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        _ => {
            eprintln!("usage: cargo xtask <perf|bundle|bundle-msi|fmt|lint>");
            ExitCode::from(2)
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(program: &str, args: &[&str]) -> ExitCode {
    match Command::new(program)
        .args(args)
        .current_dir(root())
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("{program} exited with {status}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("failed to run {program}: {err}");
            ExitCode::FAILURE
        }
    }
}

fn perf() -> ExitCode {
    if run("cargo", &["build", "--release", "-p", "droplink-app"]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    let binary = root().join("target/release/droplink");
    let size = match std::fs::metadata(&binary) {
        Ok(metadata) => metadata.len(),
        Err(err) => {
            eprintln!("cannot stat {}: {err}", binary.display());
            return ExitCode::FAILURE;
        }
    };
    println!("binary_bytes={size} ({:.1} MiB)", size as f64 / 1_048_576.0);

    let smoke = Command::new(&binary)
        .arg("--smoke")
        .current_dir(root())
        .output();
    match smoke {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().find(|line| line.starts_with("startup_ms=")) {
                println!("{line}");
            }
            if !output.status.success() {
                eprintln!("smoke failed: {}", String::from_utf8_lossy(&output.stderr));
                return ExitCode::FAILURE;
            }
        }
        Err(err) => {
            eprintln!("cannot run smoke: {err}");
            return ExitCode::FAILURE;
        }
    }

    if size > SIZE_BUDGET {
        eprintln!("binary exceeds the 22 MiB budget");
        return ExitCode::FAILURE;
    }

    let hash = run(
        "cargo",
        &[
            "bench",
            "-p",
            "droplink-core",
            "--bench",
            "hash",
            "--",
            "--sample-size",
            "10",
        ],
    );
    if hash != ExitCode::SUCCESS {
        return hash;
    }

    run(
        "cargo",
        &[
            "bench",
            "-p",
            "droplink-net",
            "--bench",
            "loopback",
            "--",
            "--sample-size",
            "10",
        ],
    )
}
