// Benchmark harness: Electron vs Reeve vs Chrome tabs.
//
// Usage:
//   cargo run --release --bin benchmark -- --scenario all --duration 120
//   cargo run --release --bin benchmark -- --scenario electron
//   cargo run --release --bin benchmark -- --scenario reeve
//   cargo run --release --bin benchmark -- --scenario chrome-tabs
//
// Notes:
// - Electron requires Node.js (`npx --yes electron`).
// - Chrome/Edge must be installed at a standard location or on PATH.
// - Reeve must be built: `cargo build --release --bin reeve`.
// - The benchmark closes only processes it started; close any existing
//   browsers before running for cleaner numbers.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TEST_URLS: &[&str] = &[
    "https://linear.app",
    "https://notion.so",
    "https://figma.com",
    "https://app.slack.com/client",
    "https://open.spotify.com",
];

// ---------------- Windows process utilities ----------------

#[cfg(windows)]
mod win_proc {
    use anyhow::Result;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    #[derive(Debug, Clone)]
    pub struct ProcEntry {
        pub pid: u32,
        pub parent_pid: u32,
        pub name: String,
    }

    pub fn list_processes() -> Result<Vec<ProcEntry>> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut result = Vec::new();
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let name_end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
                    result.push(ProcEntry {
                        pid: entry.th32ProcessID,
                        parent_pid: entry.th32ParentProcessID,
                        name,
                    });
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
            Ok(result)
        }
    }

    pub fn working_set(pid: u32) -> Option<u64> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            if handle.is_invalid() {
                return None;
            }
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            let result = GetProcessMemoryInfo(handle, &mut counters, counters.cb);
            let _ = CloseHandle(handle);
            if result.is_err() {
                return None;
            }
            Some(counters.WorkingSetSize as u64)
        }
    }

    pub fn terminate(pid: u32) {
        unsafe {
            if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                if !handle.is_invalid() {
                    let _ = TerminateProcess(handle, 1);
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod win_proc {
    use anyhow::Result;
    #[derive(Debug, Clone)]
    pub struct ProcEntry {
        pub pid: u32,
        pub parent_pid: u32,
        pub name: String,
    }
    pub fn list_processes() -> Result<Vec<ProcEntry>> {
        Ok(vec![])
    }
    pub fn working_set(_pid: u32) -> Option<u64> {
        None
    }
    pub fn terminate(_pid: u32) {}
}

// ---------------- Scenario definitions ----------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scenario {
    Electron,
    Reeve,
    ChromeTabs,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::Electron => "electron",
            Scenario::Reeve => "reeve",
            Scenario::ChromeTabs => "chrome-tabs",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "electron" => Some(Scenario::Electron),
            "reeve" => Some(Scenario::Reeve),
            "chrome" | "chrome-tabs" | "chrometabs" => Some(Scenario::ChromeTabs),
            _ => None,
        }
    }
    fn name_matches(self, exe: &str) -> bool {
        let e = exe.to_lowercase();
        match self {
            Scenario::Electron => e == "electron.exe",
            Scenario::Reeve => e == "reeve.exe" || e == "msedgewebview2.exe",
            Scenario::ChromeTabs => e == "chrome.exe" || e == "msedge.exe",
        }
    }
}

// ---------------- Report types ----------------

#[derive(Serialize)]
struct Sample {
    t_secs: u64,
    total_bytes: u64,
    process_count: u32,
    processes: Vec<ProcessRow>,
}

#[derive(Serialize)]
struct ProcessRow {
    pid: u32,
    name: String,
    memory_bytes: u64,
}

#[derive(Serialize)]
struct ScenarioResult {
    scenario: String,
    duration_secs: u64,
    baseline_processes: usize,
    peak_bytes: u64,
    peak_processes: u32,
    steady_bytes: u64,
    steady_processes: u32,
    samples: Vec<Sample>,
}

// ---------------- Helpers ----------------

fn snapshot_pids() -> HashSet<u32> {
    win_proc::list_processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.pid)
        .collect()
}

fn tracked_pids(scenario: Scenario, baseline: &HashSet<u32>) -> Vec<(u32, String, u64)> {
    let all = win_proc::list_processes().unwrap_or_default();

    // Seed: any new process whose exe name matches the scenario.
    let mut tracked: HashSet<u32> = all
        .iter()
        .filter(|p| !baseline.contains(&p.pid) && scenario.name_matches(&p.name))
        .map(|p| p.pid)
        .collect();

    // Expand: add descendants of tracked PIDs (WebView2 children, GPU helpers, etc).
    let mut changed = true;
    while changed {
        changed = false;
        for p in &all {
            if !tracked.contains(&p.pid)
                && !baseline.contains(&p.pid)
                && tracked.contains(&p.parent_pid)
            {
                tracked.insert(p.pid);
                changed = true;
            }
        }
    }

    all.iter()
        .filter(|p| tracked.contains(&p.pid))
        .filter_map(|p| win_proc::working_set(p.pid).map(|m| (p.pid, p.name.clone(), m)))
        .collect()
}

fn find_chrome() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for p in candidates {
        if Path::new(p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

fn find_reeve() -> Option<PathBuf> {
    let candidates = [
        "target\\release\\reeve.exe",
        "target\\debug\\reeve.exe",
    ];
    for p in candidates {
        if Path::new(p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

// ---------------- Launchers ----------------

fn launch_electron() -> Result<()> {
    let tmp = env::temp_dir().join("reeve-bench-electron");
    fs::create_dir_all(&tmp)?;
    fs::write(
        tmp.join("package.json"),
        r#"{"name":"reeve-bench","version":"1.0.0","main":"main.js"}"#,
    )?;
    fs::write(
        tmp.join("main.js"),
        r#"
const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => {
  const win = new BrowserWindow({ width: 1100, height: 750 });
  win.loadURL(process.env.TARGET_URL);
});
"#,
    )?;

    for url in TEST_URLS {
        Command::new("npx.cmd")
            .args(["--yes", "electron", tmp.to_str().unwrap()])
            .env("TARGET_URL", url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn npx electron for {url}"))?;
    }
    Ok(())
}

fn launch_reeve() -> Result<()> {
    let bin = find_reeve().ok_or_else(|| anyhow!("reeve.exe not found; run `cargo build --release` first"))?;
    Command::new(&bin)
        .env("REEVE_BENCH_AUTO_LAUNCH", TEST_URLS.join(","))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {}", bin.display()))?;
    Ok(())
}

fn launch_chrome_tabs() -> Result<()> {
    let bin = find_chrome().ok_or_else(|| anyhow!("Chrome/Edge not found at a standard path"))?;
    let mut cmd = Command::new(&bin);
    cmd.arg("--new-window");
    for url in TEST_URLS {
        cmd.arg(url);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().with_context(|| format!("failed to spawn {}", bin.display()))?;
    Ok(())
}

fn launch(scenario: Scenario) -> Result<()> {
    match scenario {
        Scenario::Electron => launch_electron(),
        Scenario::Reeve => launch_reeve(),
        Scenario::ChromeTabs => launch_chrome_tabs(),
    }
}

// ---------------- Harness ----------------

fn run_scenario(scenario: Scenario, duration: Duration) -> Result<ScenarioResult> {
    println!("=== {} ===", scenario.name());
    let baseline = snapshot_pids();
    println!("baseline processes: {}", baseline.len());

    launch(scenario)?;
    println!("launched; warming up for 30s...");
    thread::sleep(Duration::from_secs(30));

    let start = Instant::now();
    let mut samples = Vec::new();
    let sample_interval = Duration::from_secs(5);

    while start.elapsed() < duration {
        let procs = tracked_pids(scenario, &baseline);
        let total: u64 = procs.iter().map(|(_, _, m)| m).sum();
        let count = procs.len() as u32;
        samples.push(Sample {
            t_secs: start.elapsed().as_secs(),
            total_bytes: total,
            process_count: count,
            processes: procs
                .iter()
                .map(|(pid, name, mem)| ProcessRow {
                    pid: *pid,
                    name: name.clone(),
                    memory_bytes: *mem,
                })
                .collect(),
        });
        println!(
            "  t={:>3}s  {:>6.1} MB  {} processes",
            start.elapsed().as_secs(),
            total as f64 / 1_048_576.0,
            count
        );
        thread::sleep(sample_interval);
    }

    // Kill everything we tracked.
    println!("cleaning up...");
    let final_procs = tracked_pids(scenario, &baseline);
    for (pid, _, _) in final_procs {
        win_proc::terminate(pid);
    }
    thread::sleep(Duration::from_secs(3));

    let peak_bytes = samples.iter().map(|s| s.total_bytes).max().unwrap_or(0);
    let peak_processes = samples.iter().map(|s| s.process_count).max().unwrap_or(0);

    // Steady = median of the last 3 samples.
    let mut tail: Vec<u64> = samples.iter().rev().take(3).map(|s| s.total_bytes).collect();
    tail.sort();
    let steady_bytes = tail.get(tail.len() / 2).copied().unwrap_or(0);
    let steady_processes = samples
        .iter()
        .rev()
        .take(3)
        .map(|s| s.process_count)
        .max()
        .unwrap_or(0);

    Ok(ScenarioResult {
        scenario: scenario.name().to_string(),
        duration_secs: duration.as_secs(),
        baseline_processes: baseline.len(),
        peak_bytes,
        peak_processes,
        steady_bytes,
        steady_processes,
        samples,
    })
}

// ---------------- Report ----------------

fn write_report(results: &[ScenarioResult], out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let json_path = out_dir.join(format!("benchmark-{stamp}.json"));
    fs::write(&json_path, serde_json::to_string_pretty(results)?)?;

    let md_path = out_dir.join(format!("benchmark-{stamp}.md"));
    let mut md = String::new();
    md.push_str("# Reeve benchmark\n\n");
    md.push_str(&format!(
        "Test apps: {}\n\n",
        TEST_URLS.join(", ")
    ));
    md.push_str(&format!(
        "Measured at {} (UNIX).\n\n",
        stamp
    ));

    md.push_str("## Summary\n\n");
    md.push_str("| Scenario | Peak memory | Steady memory | Peak processes |\n");
    md.push_str("|---|---|---|---|\n");

    let electron_peak = results
        .iter()
        .find(|r| r.scenario == "electron")
        .map(|r| r.peak_bytes)
        .unwrap_or(0);

    for r in results {
        let ratio = if electron_peak > 0 && r.scenario != "electron" {
            let saved = 1.0 - (r.peak_bytes as f64 / electron_peak as f64);
            format!(" ({:.0}% vs Electron)", saved * 100.0)
        } else {
            String::new()
        };
        md.push_str(&format!(
            "| {} | {:.1} MB | {:.1} MB | {}{} |\n",
            r.scenario,
            r.peak_bytes as f64 / 1_048_576.0,
            r.steady_bytes as f64 / 1_048_576.0,
            r.peak_processes,
            ratio
        ));
    }

    md.push_str("\n## Per-scenario process breakdown (last sample)\n\n");
    for r in results {
        md.push_str(&format!("### {}\n\n", r.scenario));
        if let Some(last) = r.samples.last() {
            md.push_str("| PID | Name | Memory |\n|---|---|---|\n");
            for p in &last.processes {
                md.push_str(&format!(
                    "| {} | {} | {:.1} MB |\n",
                    p.pid,
                    p.name,
                    p.memory_bytes as f64 / 1_048_576.0
                ));
            }
            md.push('\n');
        }
    }

    fs::write(&md_path, md)?;
    println!();
    println!("Wrote {}", md_path.display());
    println!("Wrote {}", json_path.display());
    Ok(())
}

// ---------------- CLI ----------------

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut scenario_arg = "all".to_string();
    let mut duration_secs: u64 = 120;
    let mut out_dir = PathBuf::from("benchmark-results");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" | "-s" => {
                scenario_arg = args.get(i + 1).cloned().unwrap_or(scenario_arg);
                i += 2;
            }
            "--duration" | "-d" => {
                duration_secs = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(duration_secs);
                i += 2;
            }
            "--out" | "-o" => {
                if let Some(s) = args.get(i + 1) {
                    out_dir = PathBuf::from(s);
                }
                i += 2;
            }
            "--help" | "-h" => {
                println!("Usage: benchmark [--scenario electron|reeve|chrome-tabs|all] [--duration SECS] [--out DIR]");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                return Ok(());
            }
        }
    }

    let scenarios: Vec<Scenario> = if scenario_arg == "all" {
        vec![Scenario::Electron, Scenario::Reeve, Scenario::ChromeTabs]
    } else {
        vec![
            Scenario::parse(&scenario_arg)
                .ok_or_else(|| anyhow!("unknown scenario: {scenario_arg}"))?,
        ]
    };

    println!("Running {} scenario(s), {}s each", scenarios.len(), duration_secs);
    println!("Close all browsers before continuing.");
    println!();

    let mut results = Vec::new();
    for sc in scenarios {
        match run_scenario(sc, Duration::from_secs(duration_secs)) {
            Ok(r) => results.push(r),
            Err(e) => eprintln!("scenario {} failed: {e:?}", sc.name()),
        }
        // Cool-down between scenarios so cleanup settles.
        thread::sleep(Duration::from_secs(5));
    }

    write_report(&results, &out_dir)?;
    Ok(())
}