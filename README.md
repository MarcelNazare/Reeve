# Reeve

**One engine. Many apps. Governed.**

Reeve is a native PWA launcher for Windows. It replaces heavyweight Electron bundles and Chromium-per-app patterns with a single shared WebView2 runtime, kernel-enforced memory caps, and a capability-gated bridge that gives web apps native superpowers without sacrificing the sandbox.

---

## The Problem

Every Electron app ships its own Chromium. Open five apps, and you are running five browser process trees. The RAM cost is enormous, startup is slow, and none of those bundled browsers can share a renderer or GPU process with each other.

```
Electron model:              Reeve model:
┌──────────┐                 ┌──────────┐ ┌──────────┐ ┌──────────┐
│ Chromium │                 │ Renderer │ │ Renderer │ │ Renderer │
│ + App A  │                 │ App A    │ │ App B    │ │ App C    │
└──────────┘                 └──────────┘ └──────────┘ └──────────┘
┌──────────┐                          ▲          ▲          ▲
│ Chromium │                          └──────────┴──────────┘
│ + App B  │                            shared WebView2
└──────────┘                            browser process
┌──────────┐
│ Chromium │                 ┌──────────────────────┐
│ + App C  │                 │  Job Object (caps)   │
└──────────┘                 └──────────────────────┘
```

Five Electron apps: five full Chromium trees.
Five Reeve apps: one browser process, five renderers, one resource budget.

---

## The Solution

Reeve is built on three pillars:

**1. Extreme resource efficiency.** Every app runs inside a single shared WebView2 environment. One browser process, one GPU process, N isolated renderers. The launcher itself is a lightweight Rust process — no bundled Chromium, no Node runtime.

**2. A native bridge with real capabilities.** Web apps request file-system access, native notifications, and other desktop features through a capability-gated IPC bridge. Every call is validated against a per-app manifest — capability check, scope check, origin check — before the native function executes. Nothing is granted by default.

**3. A managed dashboard.** Live memory, editable permission grants, per-app kill switch, and deep telemetry. The savings are visible; the control is real.

---

## Why It Matters

**For users.** Old laptops feel new again. Battery life extends because you are not running five browser engines. Memory is capped at the kernel level, so one runaway app cannot starve the others. When something misbehaves, the dashboard shows it and stops it in one click.

**For developers.** Build one PWA that works everywhere. Get desktop-grade capabilities through a predictable, documented bridge — without bundling Chromium and without learning a new framework.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Reeve supervisor (Rust process)                        │
│  ├─ App catalog (SQLite)                                │
│  ├─ Permission store + capability checks                │
│  ├─ Job Object manager (kernel memory caps)             │
│  ├─ Popup classifier + system-browser routing           │
│  ├─ PWA manifest ingestion (icons, names)               │
│  ├─ Launcher WebView (dogfoods the same environment)    │
│  └─ Dashboard WebView (same environment, live stats)    │
└──────────────────────┬──────────────────────────────────┘
                       │ one shared WebContext
┌──────────────────────▼──────────────────────────────────┐
│  WebView2 browser process (shared)                      │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                 │
│  │Renderer  │ │Renderer  │ │Renderer  │                 │
│  │Linear    │ │Notion    │ │Figma     │                 │
│  └──────────┘ └──────────┘ └──────────┘                 │
└─────────────────────────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│  Windows Job Object                                     │
│  · ProcessMemoryLimit (per-renderer cap)                │
│  · JobMemoryLimit     (total cap)                       │
│  · KillOnJobClose     (no orphaned renderers)           │
└─────────────────────────────────────────────────────────┘
```

Every renderer is attached to the Job Object via `ICoreWebView2Environment13::GetProcessInfos`, rescanned every 3 seconds. Memory limits are enforced by the Windows kernel. WebView2 sandboxes its own renderers into a private job, so some `AssignProcessToJobObject` calls return `ERROR_ACCESS_DENIED` — expected and handled gracefully; the job-wide cap on the browser process covers the tree.

---

## Current State

**Working end-to-end:**

- ✅ **Launcher UI:** Modern dark theme, search bar with URL-based install, and app grid.
- ✅ **Manifest Ingestion:** Fetches `manifest.json` for real app names and locally caches high-res icons.
- ✅ **Shared Runtime:** One WebView2 browser process across all apps.
- ✅ **Job Object Governor:** Kernel-enforced per-process and job-wide memory caps.
- ✅ **Capability Bridge:** `fs.pick`, `fs.read`, `fs.write`, `notify` with canonicalized path checks and origin gating.
- ✅ **Smart Popup Handling:** OAuth and same-brand popups open as managed windows; external links route to the system browser.
- ✅ **Managed Dashboard:** Live memory, editable permissions, kill switch.
- ✅ **Deep Telemetry:** Per-app memory attribution via frame mapping and `sessions.peak_mem_bytes` from live Job Object stats.
- ✅ **Benchmark Harness:** Repeatable cold-start and steady-state memory measurement (Electron vs Reeve vs Chrome tabs).
- ✅ **Embedded `.exe` icon** (`winres`).

**Not yet implemented:**

- ⏳ macOS and Linux targets
- ⏳ Toolhelp32 fallback for older WebView2 runtimes

---

## Requirements

- **Windows 10 or 11** (WebView2 Runtime pre-installed on all modern builds)
- **Rust 1.75+** (stable toolchain)
- **Visual Studio Build Tools** with the C++ workload (MSVC linker)
- **GNU Make** — ships with Git for Windows and MSYS2; install via `choco install make` or `scoop install make` if missing
- **Node.js** (only for the Electron benchmark scenario)

Verify WebView2 is present:

```powershell
Get-ItemProperty -Path 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F1E7E9DA-3F7D-4A15-A8D0-1C3A9F0F6C6C}' -Name pv -ErrorAction SilentlyContinue
```

If it returns nothing, install the [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).

---

## Building & Running

```bash
git clone https://github.com/MarcelNazare/Reeve.git
cd Reeve
cargo check
cargo run
```

Or with the bundled `Makefile`:

```bash
make          # build
make run      # build + run with debug logging
make check    # fast type-check
make release  # optimized build (LTO, stripped)
make clean    # remove target/
```

**Make on Windows:** run `make` from **Git Bash** or **MSYS2**, not `cmd.exe`. The `run` target sets `RUST_LOG` with POSIX shell syntax.

Data directory:

```
%APPDATA%\Reeve\Reeve\data\
├── catalog.db      # apps, permissions, sessions
├── icons\          # cached PWA icons from manifest.json
└── webview\        # shared WebView2 user-data folder
```

---

## Adding Apps

### Via the launcher
Paste a URL into the search bar and press **Add**. Reeve immediately records the app, then spawns a background thread that:
1. Fetches the page HTML and locates `<link rel="manifest">`.
2. Parses the `manifest.json`.
3. Extracts the real app name.
4. Downloads the highest-resolution icon and caches it locally in `data/icons/`.
5. Updates the grid automatically when finished. (Falls back to Google favicons if no manifest exists).

### Via seeding
Edit `seed_catalog()` in `src/main.rs` and add:
```rust
catalog.add_app("myapp", "My App", "https://myapp.example")?;
```
Seeding runs on every launch and is idempotent (`INSERT OR REPLACE` on `id`, `INSERT OR IGNORE` on permissions). New entries appear on the next launch.

---

## Dashboard

Click **Dashboard** in the launcher header. A second window opens, sharing the same WebView2 environment.

### Live Memory & Telemetry
Every 5 seconds, Reeve queries `ICoreWebView2Environment13::GetProcessExtendedInfos` to map renderer PIDs to specific app frames via their source URLs.
- **Total now:** Sum of working set across the entire tree.
- **Peak job memory:** Highest committed memory ever seen by the Windows Job Object (`PeakJobMemoryUsed`).
- **Per-app Memory:** Attributed by matching renderer frames to the app's `start_url`.
- **Peak (session):** Highest observed memory for the app's current session. Written to `sessions.peak_mem_bytes` in the database when the window is closed or killed.

### Permission Editor
Each app row shows its grants as chips. Click **×** to revoke a specific `(capability, scope)` pair. Click **+ Add** to grant `notify` / `fs.pick` / `fs.read` / `fs.write` with an optional path scope. Grants take effect on the app's next launch.

### Kill Switch
Each open app row shows **Kill**, which drops its `Window` and `WebView` (closing the OS window), transitions the instance to `Lifecycle::Killed`, writes the session peak, and ends the SQLite session.

---

## Benchmark

The harness in `src/bin/benchmark.rs` runs configurations in isolation. Run it with:

```bash
# Full suite (Electron + Reeve + Chrome tabs)
make bench

# Quick smoke test (60s per scenario)
make bench-quick

# Single scenario
make bench-reeve
```

### Methodology
- **Five test apps:** Linear, Notion, Figma, Slack, Spotify.
- **Cold start (`t_stable`):** Polled every 100ms. Measures the time until total tree memory has been flat (< 5 MB growth) for 1.5s while > 50 MB.
- **Cold start (`t_ready`):** Measured from the exact moment the first app's page finishes loading (Reeve only, via `on_page_load`).
- **Steady state:** 30s warm-up, then samples `WorkingSet64` every 5s for the configured duration.
- **Isolation:** Only one scenario runs at a time. Kills only processes it spawned.

### Latest Results (Reeve vs Chrome Tabs)

| Metric | Reeve | Chrome Tabs |
|---|---|---|
| **Cold start (t_stable)** | 41,100 ms | 9,200 ms |
| **Cold start (t_ready)** | 12,460 ms | — |
| **Peak memory** | 2575.5 MB | 2212.2 MB |
| **Steady memory** | 2165.5 MB | 2195.3 MB |
| **Peak processes** | 18 | 24 |

**What this tells us:**

1. **Steady-state memory is at parity.** Reeve (2165 MB) and Chrome (2195 MB) use essentially the same amount of RAM for these five heavy apps. The "80% less RAM" claim holds against Electron (which typically uses 2x-3x more), but **not** against Chrome tabs. We report this honestly.
2. **Process count is lower.** Reeve achieves parity with 18 processes vs Chrome's 24, because WebView2 consolidates more of the browser tree.
3. **Cold start is slower for Reeve.** `t_ready` (first page loaded) is 12.4s, but `t_stable` (all background WebView2 initialization complete) takes 41.1s. Chrome reaches stability in 9.2s. WebView2's background GPU/utility process spin-up is the bottleneck. We are investigating lazy initialization of the shared environment.
4. **Electron (not shown in this run)** typically peaks over 4,000 MB for this same set of apps, because each app spawns its own independent Chromium instance.

---

## Capability Model

Every app declares which native capabilities it needs. Grants live in `(app_id, capability, scope)`.

| Capability | Scope | Native implementation |
|---|---|---|
| `fs.pick` | none (user-mediated) | `rfd::FileDialog::pick_files` |
| `fs.read` | path prefix | `std::fs::read_to_string` (canonicalized) |
| `fs.write` | path prefix | `std::fs::write` (canonicalized) |
| `notify` | none | `notify_rust::Notification::show` |

Both the requested path and the granted scope are canonicalized before the prefix check, blocking `..` traversal and symlink escapes.

---

## How the Bridge Works

From a web app's JavaScript:

```js
window.ipc.postMessage(JSON.stringify({
  capability: "fs.read",
  args: { path: "C:\\Users\\Public\\notes.txt" },
  request_id: "req-1"
}));

window.addEventListener("reeve-response", (e) => {
  if (e.detail.request_id === "req-1") {
    if (e.detail.ok) console.log("contents:", e.detail.data.content);
    else console.error("denied:", e.detail.error);
  }
});
```

Every call passes through three gates:
1. **Origin gate**: IPC page host must match the app's launch origin.
2. **Capability gate**: App must have an entry for that capability.
3. **Scope gate**: Canonicalized requested path must fall within a canonicalized granted scope.

---

## Popup Handling

Every app window installs a `new_window_req_handler`. The classifier (`classify_popup`) routes popups through these gates:
- **Opaque scheme** (`about:`, `blob:`, `data:`) → Managed
- **Same origin** (or subdomain) → Managed
- **Same brand** (`notion.so` ↔ `app.notion.com`) → Managed
- **Known OAuth** (`accounts.google.com`, etc.) → Managed
- **OAuth query parameters** (`redirect_uri=`, `clientId=`, `popupFlowId=`) → Managed
- **Everything else** → System browser

Managed popups open via `NewWindowResponse::Allow` as real WebView2 windows sharing the parent's cookie jar. External links route to the default browser via `open::that` and are denied from loading inside the app.

---

## Project Layout

```
reeve/
├── Cargo.toml
├── Makefile
├── build.rs                # winres: embeds assets/reeve.ico
├── README.md
├── LICENSE                 # MIT
├── .gitignore
├── assets/
│   └── reeve.ico           # multi-resolution Windows icon
├── migrations/
│   └── 001_init.sql
├── benchmark-results/      # generated by the benchmark harness
└── src/
    ├── main.rs             # entry, seed_catalog(), launches the UI
    ├── catalog.rs          # SQLite: apps, permissions, sessions
    ├── environment.rs      # shared WebView2 data directory
    ├── governor.rs         # Job Object, memory queries, process snapshots
    ├── bridge.rs           # capability checks + native dispatch
    ├── instance.rs         # app lifecycle state machine
    ├── manifest.rs         # PWA manifest ingestion (fetch, parse, download icon)
    ├── ui.rs               # launcher, dashboard, IPC, popups, URL install
    └── bin/
        └── benchmark.rs    # Electron vs Reeve vs Chrome tabs harness
```

---

## Make Targets Reference

| Command | What it does |
|---|---|
| `make` / `make all` | Alias for `make build` |
| `make build` | `cargo build` (debug) |
| `make release` | `cargo build --release` (LTO, stripped) |
| `make run` | `cargo run` with `RUST_LOG=info,reeve=debug` |
| `make check` | `cargo check` (fast type-check, no codegen) |
| `make test` | `cargo test` |
| `make fmt` | `cargo fmt` |
| `make clippy` | `cargo clippy -- -D warnings` |
| `make bench` | Full benchmark suite (120s per scenario) |
| `make bench-quick` | Quick benchmark (60s per scenario) |
| `make bench-long` | Publication benchmark (600s per scenario) |
| `make bench-reeve` | Benchmark Reeve only |
| `make bench-electron` | Benchmark Electron only |
| `make bench-chrome` | Benchmark Chrome/Edge only |
| `make clean` | `cargo clean` |
| `make distclean` | `cargo clean` + remove `benchmark-results/` |

---

## Roadmap

| Milestone | Status |
|---|---|
| Shared environment, one browser process | ✅ |
| Job Object memory caps, renderer attachment | ✅ |
| Capability-gated bridge (v1 capabilities) | ✅ |
| Origin gate on IPC | ✅ |
| Bridge response delivery with `request_id` | ✅ |
| Session tracking on open/close | ✅ |
| URL search bar with dedupe | ✅ |
| Popup classification (OAuth, same-brand, query heuristic) | ✅ |
| Embedded `.exe` icon + per-app favicons | ✅ |
| Dashboard: live memory, permission editor, kill switch | ✅ |
| PWA manifest ingestion (icon, name from manifest.json) | ✅ |
| Per-app memory attribution | ✅ |
| `sessions.peak_mem_bytes` from live Job Object stats | ✅ |
| Benchmark harness (Electron vs Reeve vs Chrome tabs) | ✅ |
| Cold-start benchmark | ✅ |
| macOS target (WKWebView, soft memory limits) | ⏳ |
| Linux target (WebKitGTK, cgroups v2) | ⏳ |

---

## Security Model

The bridge is the attack surface. Design principles:

- **Deny by default.** No capability without an explicit grant.
- **Origin-gated IPC.** Pages on the app's declared origin (or subdomain) only.
- **Scope every path.** Both sides canonicalized; symlinks and `..` rejected.
- **User-mediated pick.** `fs.pick` uses the OS dialog — the user chooses the file.
- **Job Object isolation.** Runaway renderers are capped at the kernel level.
- **Kill on close.** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` prevents orphaned renderers.

Not a formal security audit. Known limitations to address before production:
- **Subdomain matching is naive** — `host.ends_with(".app.example")` is safe for exact hosts, but a full implementation must consult the Public Suffix List to avoid `evil.co.uk` matching `.co.uk`.
- **No rate limiting on the bridge.**
- **No audit log.**
- **Kill switch is not a security boundary.** It's a UX tool. The actual containment is the Job Object cap.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `couldn't read migrations/001_init.sql` | Confirm `migrations/001_init.sql` exists at the project root |
| `couldn't read assets/reeve.ico` | Confirm `assets/reeve.ico` exists; `build.rs` requires it |
| `ICoreWebView2Environment13` not found | Match `webview2-com` version to what `cargo tree -p wry` reports |
| `GetProcessMemoryInfo` not found | Confirm `Win32_System_ProcessStatus` feature is enabled in `Cargo.toml` |
| Benchmark crashes on ToolHelp | Add `Win32_System_Diagnostics_ToolHelp` to the `windows` features list |
| Popup flashes and vanishes | Confirm `wry = "0.55"` or later — older versions run the handler off-thread |
| Site shows "popups blocked" | The URL was denied. Check the `NewWindowRequested` log and add the host to `OAUTH_HOSTS` or extend `looks_like_oauth` |
| Renderer attachment logs `Access is denied` | Expected — WebView2 sandboxes renderers; the browser-process cap covers the tree |
| Dashboard shows 0 B memory | No apps are open yet — click one in the launcher grid first |

---

## Contributing

Reeve is in early development. The most valuable contributions right now:

1. **macOS and Linux targets.** The governance layer is stubbed on non-Windows; porting requires WKWebView + memory-pressure signals on macOS, WebKitGTK + cgroups v2 on Linux.
2. **Public Suffix List matching** for the origin gate.
3. **WebView2 cold-start optimization.** The benchmark shows `t_stable` is 4x slower than Chrome. Investigating lazy GPU initialization or deferred utility process spin-up would close this gap.

Open an issue before starting non-trivial work.

---

## License

MIT. See [LICENSE](LICENSE).

---

## Acknowledgements

Reeve stands on the shoulders of:

- **[wry](https://github.com/tauri-apps/wry)** — cross-platform WebView abstraction
- **[tao](https://github.com/tauri-apps/tao)** — windowing and event loop
- **[windows](https://github.com/microsoft/windows-rs)** — Windows API bindings, including the Job Object surface Reeve uses directly
- **[webview2-com](https://github.com/wravery/webview2-rs)** — WebView2 COM bindings
- **[winres](https://github.com/mxre/winres)** — Windows resource embedding for the executable icon
- **[reqwest](https://github.com/seanmonstar/reqwest)** and **[scraper](https://github.com/causal-agent/scraper)** — PWA manifest ingestion

The shared-runtime model was inspired by the observation that modern desktop apps are just web apps with a browser bundled inside. Reeve removes the bundle.

The dashboard's honest aggregate memory metric was a deliberate choice: WebView2 doesn't expose per-controller memory attribution natively, so Reeve derives it from frame URLs where the OS provides them. The savings are real; the dashboard proves it without lying about the granularity.