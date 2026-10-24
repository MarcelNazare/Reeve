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

**3. A managed dashboard.** Live memory per app, permission grants, session history. The savings are visible, not a marketing claim.

---

## Why It Matters

**For users.** Old laptops feel new again. Battery life extends because you are not running five browser engines. Memory is capped at the kernel level, so one runaway app cannot starve the others.

**For developers.** Build one PWA that works everywhere. Get desktop-grade capabilities through a predictable, documented bridge — without bundling Chromium and without learning a new framework.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Reeve supervisor (Rust process)                        │
│  ├─ App catalog (SQLite)                                │
│  ├─ Permission store + capability checks                │
│  ├─ Job Object manager (kernel memory caps)             │
│  └─ Launcher WebView (dogfoods the same environment)    │
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

Every renderer is attached to the Job Object within 3 seconds of spawn via `ICoreWebView2Environment13::GetProcessInfos`. Memory limits are enforced by the Windows kernel — when a renderer tries to commit past its budget, the allocation fails. Reeve does not poll RSS and react after the fact.

**Implementation note:** the governor talks to the Windows Job Object API **directly through the `windows` crate** (`CreateJobObjectW` + `SetInformationJobObject` + `AssignProcessToJobObject`). It does **not** use `win32job` — that crate only wraps working-set limits, not the committed-memory limits this project needs.

---

## Current State

This repository is a **working end-to-end application**. What is implemented:

- ✅ Launcher UI with app grid, seeded catalog, IPC-driven app launch
- ✅ Shared WebView2 environment across all app windows (one browser process)
- ✅ Job Object governor with per-process and job-wide committed-memory caps, created via the `windows` crate directly
- ✅ Renderer enumeration and attachment via `GetProcessInfos`, rescanning every 3 seconds
- ✅ Capability-gated native bridge for `fs.pick`, `fs.read`, `fs.write`, `notify`
- ✅ Per-app permission storage with canonicalized path checks (blocks `..` and symlink escapes)
- ✅ **Origin gate** — IPC rejected unless the page host matches the app's launch origin
- ✅ **Bridge response delivery** through the event loop, with `request_id` correlation
- ✅ Session tracking (start on spawn, end on window close)
- ✅ SQLite catalog for apps, permissions, and sessions

What is not yet implemented:

- ⏳ Dashboard view (memory table, permission editor, kill switch)
- ⏳ PWA manifest ingestion (installer flow — apps are seeded from code today)
- ⏳ Benchmark harness for the RAM comparison
- ⏳ macOS and Linux targets (structure is in place; native primitives are stubbed)
- ⏳ Populating `sessions.peak_mem_bytes` from live Job Object stats
- ⏳ Toolhelp32 fallback for WebView2 runtimes older than ~2023

---

## Requirements

- **Windows 10 or 11** (WebView2 Runtime pre-installed on all modern builds)
- **Rust 1.75+** (stable toolchain)
- **Visual Studio Build Tools** with the C++ workload (for the MSVC linker)
- **GNU Make** — ships with Git for Windows (via Git Bash) and MSYS2. Install via `choco install make` or `scoop install make` if missing.

Verify WebView2 is present:

```powershell
Get-ItemProperty -Path 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F1E7E9DA-3F7D-4A15-A8D0-1C3A9F0F6C6C}' -Name pv -ErrorAction SilentlyContinue
```

If it returns nothing, install the [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).

---

## Building

```bash
git clone https://github.com/<your-org>/reeve.git
cd reeve
cargo check
cargo run
```

Or with the bundled `Makefile` (recommended — it sets the debug log filter):

```bash
make          # build
make run      # build + run with debug logging
make check    # fast type-check
make test     # run tests
make release  # optimized build (LTO, stripped)
make clean    # remove target/
make fmt      # format code
make clippy   # lint with warnings as errors
```

**Make on Windows:** run `make` from **Git Bash** or **MSYS2**, not `cmd.exe`. The `run` target sets `RUST_LOG` with POSIX shell syntax. From PowerShell, edit the `run` target to `$$env:RUST_LOG="info,reeve=debug"; $(CARGO) run`.

The launcher opens with three seeded apps (Linear, Notion, Figma). Click any app to spawn a window in the shared environment. Open all three, then check Task Manager → Details: there should be **one** `msedgewebview2.exe` browser process, plus one renderer per app. That is the RAM story, proven.

Data directory (catalog, WebView2 user-data, logs):

```
%APPDATA%\Reeve\Reeve\data\
├── catalog.db
└── webview\
```

Logging is on by default at `info`; set `RUST_LOG=debug` for bridge-level tracing:

```powershell
$env:RUST_LOG="info,reeve=debug"
cargo run
```

`make run` does this for you.

---

## Project Layout

```
reeve/
├── Cargo.toml
├── Makefile
├── README.md
├── .gitignore
├── migrations/
│   └── 001_init.sql
└── src/
    ├── main.rs           # entry point, seeds catalog, launches UI
    ├── catalog.rs        # SQLite: apps, permissions, sessions
    ├── environment.rs    # shared WebView2 data directory
    ├── governor.rs       # Job Object (via windows crate) + renderer enumeration
    ├── bridge.rs         # capability checks + native dispatch
    ├── instance.rs       # app lifecycle state machine
    └── ui.rs             # launcher window, IPC routing, window spawning
```

> `migrations/001_init.sql` **must exist at the project root**, next to `Cargo.toml`. `src/catalog.rs` includes it via `include_str!("../migrations/001_init.sql")`, and the build fails without it.

---

## Make Targets Reference

| Command | What it does |
|---|---|
| `make` / `make all` | Alias for `make build` |
| `make build` | `cargo build` (debug) |
| `make run` | `cargo run` with `RUST_LOG=info,reeve=debug` |
| `make check` | `cargo check` (fast type-check, no codegen) |
| `make test` | `cargo test` |
| `make fmt` | `cargo fmt` |
| `make clippy` | `cargo clippy -- -D warnings` |
| `make release` | `cargo build --release` (LTO, stripped) |
| `make clean` | `cargo clean` |

---

## Capability Model

Every app declares which native capabilities it needs. The catalog stores grants per `(app_id, capability, scope)`. Permissions are preloaded into the bridge when an app window is created; changing grants while an app is running requires restarting that app's window.

| Capability | Scope | Native implementation |
|---|---|---|
| `fs.pick` | none (user-mediated dialog) | `rfd::FileDialog::pick_files` |
| `fs.read` | path prefix | `std::fs::read_to_string` (canonicalized) |
| `fs.write` | path prefix | `std::fs::write` (canonicalized) |
| `notify` | none | `notify_rust::Notification::show` |

Grants are seeded programmatically today. A future milestone exposes a permission editor in the dashboard.

Example grant (from `main.rs`):

```rust
catalog.grant_permission("notion", "fs.read",  r"C:\Users\Public")?;
catalog.grant_permission("notion", "fs.write", r"C:\Users\Public")?;
catalog.grant_permission("linear", "notify",   "")?;
```

The bridge canonicalizes both the requested path and the granted scope before the prefix check, blocking `..` traversal and symlink escapes.

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

Every call passes through three gates before the native function runs:

1. **Origin gate** — the IPC event's page host must match the app's launch origin (or a subdomain of it). Blocks third-party pages from invoking the bridge during OAuth redirects or malicious navigations.
2. **Capability gate** — the app must have an entry for that capability in the `permissions` table.
3. **Scope gate** — for path-taking capabilities, the canonicalized requested path must fall within a canonicalized granted scope.

A failed check returns a structured response and never touches the OS:

```json
{ "ok": false, "error": "path 'C:\\Windows' is outside the granted scope for 'fs.read'", "request_id": "req-1" }
```

Responses are delivered back to the app's JavaScript through the Tao event loop — `UserEvent::BridgeResponse` carries a script that dispatches a `reeve-response` `CustomEvent` on `window`. This avoids the deadlock that occurs when trying to `evaluate_script` from inside the IPC handler during webview construction.

---

## Renderer Enumeration

The `governor` module uses `ICoreWebView2Environment13::GetProcessInfos`, which returns every process associated with the shared WebView2 environment, tagged by kind (`Browser`, `Renderer`, `Gpu`, `Utility`, `Crashpad`). Only `Renderer` processes are attached to the Job Object; browser, GPU, and Crashpad processes are left alone.

Why not walk the process tree with `CreateToolhelp32Snapshot`? Because WebView2's process tree is not a simple parent/child hierarchy, and renderer PIDs change as pages navigate. The environment owns the authoritative list.

The event loop rescans every 3 seconds and attaches any newly spawned renderers — pages fork additional renderers as their workload grows (Notion opening a large doc, Figma loading a canvas), and each new renderer must be capped.

---

## Roadmap

| Milestone | Status |
|---|---|
| Shared environment, one browser process | ✅ |
| Job Object memory caps, renderer attachment (via `windows` crate) | ✅ |
| Capability-gated bridge (v1 capabilities) | ✅ |
| Origin gate on IPC | ✅ |
| Bridge response delivery with `request_id` | ✅ |
| Session tracking on open/close | ✅ |
| Makefile build helpers | ✅ |
| Dashboard (memory table, permission editor, kill switch) | ⏳ |
| PWA manifest installer | ⏳ |
| Live `peak_mem_bytes` from Job Object stats | ⏳ |
| Benchmark harness (Electron vs Reeve vs Chrome tabs) | ⏳ |
| macOS target (WKWebView, soft memory limits) | ⏳ |
| Linux target (WebKitGTK, cgroups v2) | ⏳ |

---

## The Benchmark We Owe You

The "80% less RAM" claim will be published with full methodology. The plan:

1. Cold-start the same five apps (Linear, Notion, Figma, Slack, Spotify) as **Electron apps**, as **Reeve PWAs**, and as **five Chrome tabs**.
2. Measure peak working set and committed memory per configuration over 10 minutes of light interaction.
3. Publish raw numbers and the harness.

Expectation based on the architecture: Reeve will significantly beat Electron, and will be roughly at parity with Chrome tabs — but with per-app memory caps, a capability bridge, and a managed dashboard that Chrome does not offer. The honest comparison is Electron → Reeve. Chrome tabs → Reeve is a different story about control, not just memory.

---

## Security Model

The bridge is the attack surface. Design principles:

- **Deny by default.** No capability is granted without an explicit catalog entry.
- **Origin-gated IPC.** A page must be on the app's declared origin (or a subdomain) to invoke the bridge at all. Redirects and injected iframes fail the gate.
- **Scope every path.** File-system operations canonicalize both sides before comparing. Symlinks and `..` traversal are rejected.
- **User-mediated grants.** `fs.pick` uses the OS file dialog — the user chooses the file, not the web app.
- **Job Object isolation.** A runaway or malicious renderer is capped at the kernel level. It cannot exhaust system memory.
- **Kill on close.** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` guarantees no orphaned renderers if Reeve crashes.

This is not a formal security audit. Before shipping to end users, the bridge should be threat-modeled and externally reviewed — particularly the canonicalization logic, the origin comparison (subdomain matching must handle public-suffix rules), and the IPC body parser.

Known limitations to address before production:

- **Subdomain matching is naive.** `host.ends_with(".app.example")` is safe for exact hosts, but a full implementation must consult the Public Suffix List to avoid `evil.co.uk` matching `.co.uk`.
- **No rate limiting on the bridge.** A malicious app could spam `fs.pick` to annoy the user with dialogs.
- **No audit log.** Bridge calls should be logged to `sessions` for post-incident review.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `couldn't read migrations/001_init.sql` | Confirm `migrations/001_init.sql` exists at the project root, next to `Cargo.toml` |
| `no WebviewExtWindows in the root` | Trait is `WebViewExtWindows` — capital V |
| `no method named 'with_web_context'` | wry 0.52+ renamed it to `WebViewBuilder::new_with_web_context(&mut ctx)` |
| `ICoreWebView2Environment13` not found | Match `webview2-com` version to whatever `cargo tree -p wry` reports |
| `ProcessMemoryLimit` / `JobMemoryLimit` field not found | Confirm `windows = "0.58"` and that `Win32_System_JobObjects` feature is enabled |
| `assign_process` wants `HANDLE` not `u32` | Reeve does not use `win32job`; it calls `OpenProcess` + `AssignProcessToJobObject` directly |

---

## Contributing

Reeve is in early development. The most valuable contributions right now:

1. **Renderer enumeration reliability.** `GetProcessInfos` requires a recent WebView2 Runtime. Test against older builds and report which versions fail — a Toolhelp32 fallback is needed for those machines.
2. **Dashboard.** The catalog already stores everything the dashboard needs. Build the memory table, permission editor, and kill switch as another WebView sharing the same environment.
3. **Benchmark harness.** A reproducible script for the three-way comparison above.
4. **Public Suffix List matching** for the origin gate.

Open an issue before starting non-trivial work so the approach can be discussed.

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

The shared-runtime model was inspired by the observation that modern desktop apps are just web apps with a browser bundled inside. Reeve removes the bundle.