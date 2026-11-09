//! Fake Minecraft Java server for `minewrap-core` process tests.
//!
//! Ports of the Python v1 `legacy/tests/fake_mc_server.py` scripts (12 mode
//! flags over 6 fixtures). This binary prints realistic Minecraft log lines,
//! reads stdin commands, and exits cleanly on `stop`.

use std::io::{self, BufRead, Write};
use std::time::Duration;

/// Modes copied from the v1 fixture names.
pub enum Mode {
    Normal,
    CrashDuringStart,
    CrashAfterDone,
    IgnoreStop,
    IgnoreSigterm,
    DoneDelay(u64),
    Burst(u64),
}

fn main() -> std::process::ExitCode {
    let flags: Vec<String> = std::env::args().collect();
    let stdin = io::stdin();
    let lines = io::BufReader::new(stdin.lock()).lines();

    let mode = parse_mode(&flags);
    if matches!(mode, Mode::CrashDuringStart) {
        println!("[main/INFO]: Starting minecraft server version 1.21.4");
        std::thread::sleep(Duration::from_millis(50));
        eprintln!("Simulated crash");
        return std::process::ExitCode::from(1);
    }

    println!("[main/INFO]: Starting minecraft server version 1.21.4");
    println!("[main/WARN]: [FakeServer] test mode");
    std::thread::sleep(Duration::from_millis(50));
    println!("[Server thread/INFO]: Preparing level \"world\"");
    std::thread::sleep(Duration::from_millis(50));
    println!("[Server thread/INFO]: Done (1.234s)! For help, type \"help\" or \"?\"");

    if matches!(mode, Mode::CrashAfterDone) {
        std::thread::sleep(Duration::from_millis(50));
        eprintln!("Simulated crash");
        return std::process::ExitCode::from(1);
    }

    if let Mode::Burst(n) = mode {
        for i in 0..n {
            println!("[Server thread/INFO]: burst {i}");
        }
    }

    let _ = io::stdout().flush();
    loop {
        match lines.next() {
            Some(Ok(line)) => {
                let trimmed = line.trim();
                if trimmed == "stop" && !matches!(mode, Mode::IgnoreStop) {
                    println!("[Server thread/INFO]: Stopping the server");
                    println!("[Server thread/INFO]: Stopping server");
                    return std::process::ExitCode::SUCCESS;
                }
                if trimmed == "crash" {
                    eprintln!("Simulated crash by command");
                    return std::process::ExitCode::from(1);
                }
                println!("[Server thread/INFO]: Executing command: {trimmed}");
            }
            _ => return std::process::ExitCode::SUCCESS,
        }
    }
}

fn parse_mode(flags: &[String]) -> Mode {
    let has = |needle: &str| flags.iter().any(|f| f == needle);
    if has("--crash") {
        if let Some(i) = flags.iter().position(|f| f == "--crash") {
            match flags.get(i + 1).map(|s| s.as_str()) {
                Some("during-start") => return Mode::CrashDuringStart,
                Some("after-done") => return Mode::CrashAfterDone,
                _ => {}
            }
        }
    }
    if has("--ignore-stop") {
        return Mode::IgnoreStop;
    }
    Mode::Normal
}

pub fn mode_name(mode: &Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::CrashDuringStart => "crash-during-start",
        Mode::CrashAfterDone => "crash-after-done",
        Mode::IgnoreStop => "ignore-stop",
        Mode::IgnoreSigterm => "ignore-sigterm",
        Mode::DoneDelay(_) => "done-delay",
        Mode::Burst(_) => "burst",
    }
}

