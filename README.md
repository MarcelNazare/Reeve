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

**3. A managed dashboard.** Live memory per app, editable permission grants, and a per-app kill switch. The savings are visible, not a marketing claim.

---

## Why It Matters

**For users.** Old laptops feel new again. Battery life extends because you are not running five browser engines. Memory is capped at the kernel level, so one runaway app cannot starve the others. When something misbehaves, the dashboard lets you see it and stop it in one click.

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

The governor talks to the Job Object API **directly through the `windows` crate** (`CreateJobObjectW` + `SetInformationJobObject` + `AssignProcessToJobObject`), not through a third-party wrapper.

The dashboard is a second WebView in the same `WebContext`, which means it sees the entire WebView2 process tree via `GetProcessInfos` — the honest aggregate number that proves the RAM story.

---

## Current State

**Working end-to-end:**

- ✅ Launcher UI with search bar, app grid, URL-based install
- ✅ 30 seeded PWAs across productivity, communication, media, Google Workspace, dev tools, social
- ✅ Idempotent seeding — new seed entries appear on next launch without a DB wipe
- ✅ Shared WebView2 environment (one browser process across all apps)
- ✅ Job Object governor with per-process and job-wide memory caps
- ✅ Renderer enumeration via `GetProcessInfos`, best-effort attachment
- ✅ Capability-gated bridge: `fs.pick`, `fs.read`, `fs.write`, `notify`
- ✅ Canonicalized path checks (blocks `..` and symlink escapes)
- ✅ Origin gate on IPC (subdomain-aware)
- ✅ Bridge response delivery through the event loop with `request_id` correlation
- ✅ Session tracking (start on spawn, end on window close)
- ✅ Smart popup handling: OAuth and same-brand popups open as managed windows; external links route to the system browser
- ✅ URL search bar with dedup against id and start_url
- ✅ **Dashboard with live memory, editable permissions, and kill switch**
- ✅ Embedded `.exe` icon (`winres`) and per-app favicons in the grid

**Not yet implemented:**

- ⏳ PWA manifest ingestion (today the search bar only records the URL)
- ⏳ Benchmark harness for the RAM comparison
- ⏳ macOS and Linux targets
- ⏳ `sessions.peak_mem_bytes` populated from live Job Object stats
- ⏳ Toolhelp32 fallback for older WebView2 runtimes
- ⏳ Per-app memory attribution (blocked by WebView2's environment-wide API)

---

## Requirements

- **Windows 10 or 11** (WebView2 Runtime pre-installed on all modern builds)
- **Rust 1.75+** (stable toolchain)
- **Visual Studio Build Tools** with the C++ workload (MSVC linker)
- **GNU Make** — ships with Git for Windows and MSYS2; install via `choco install make` or `scoop install make` if missing

Verify WebView2 is present:

```powershell
Get-ItemProperty -Path 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F1E7E9DA-3F7D-4A15-A8D0-1C3A9F0F6C6C}' -Name pv -ErrorAction SilentlyContinue
```

If it returns nothing, install the [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).

---

## Building

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
make test     # run tests
make release  # optimized build (LTO, stripped)
make clean    # remove target/
make fmt      # format code
make clippy   # lint with warnings as errors
```

**Make on Windows:** run `make` from **Git Bash** or **MSYS2**, not `cmd.exe`. The `run` target sets `RUST_LOG` with POSIX shell syntax. From PowerShell, edit the `run` target to `$$env:RUST_LOG="info,reeve=debug"; $(CARGO) run`.

On first launch, Reeve opens with 30 seeded apps. Click one to spawn a window in the shared environment. Open five of them and check Task Manager → Details: there should be **one** `msedgewebview2.exe` browser process, plus one renderer per app.

Data directory:

```
%APPDATA%\Reeve\Reeve\data\
├── catalog.db      # apps, permissions, sessions
└── webview\        # shared WebView2 user-data folder
```

Logging is on by default at `info`; set `RUST_LOG=debug` for bridge-level tracing:

```powershell
$env:RUST_LOG="info,reeve=debug"
cargo run
```

---

## Adding Apps

### Via the launcher

Paste a URL into the search bar at the top of the launcher and press **Add**. Reeve accepts:

- `linear.app` (bare host — `https://` is prepended)
- `https://app.notion.com/some/path` (path preserved)
- `www.example.co.uk` (`www.` is stripped)

The id is derived from the host (`app.notion.com` → `app-notion-com`), the display name from the brand label (`Notion`), and duplicates are detected against both id and start_url. Success and failure are reported via a transient toast.

### Via seeding

Edit `seed_catalog()` in `src/main.rs` and add:

```rust
catalog.add_app("myapp", "My App", "https://myapp.example")?;
```

Seeding runs on every launch and is idempotent (`INSERT OR REPLACE` on `id`, `INSERT OR IGNORE` on permissions). New entries appear on the next `make run`; existing apps and search-bar additions are untouched.

---

## Dashboard

Click **Dashboard** in the launcher header. A second window opens, sharing the same WebView2 environment.

### Live memory

Every 2 seconds, the dashboard queries `ICoreWebView2Environment13::GetProcessInfos` to enumerate every process associated with the environment, then resolves each PID's working set via `GetProcessMemoryInfo`. Two displays:

- **Total WebView2 memory** — the sum of working set across browser, GPU, and every renderer. This is the number that matters for the "one engine, many apps" story.
- **Per-kind breakdown** — a chip per process kind showing its share.

Per-app memory is **deliberately not** attributed, because WebView2's environment-wide API doesn't map renderers to controllers. The dashboard tells the truth: aggregate memory is what the OS sees.

### Permission editor

Each app row shows its grants as chips. Click **×** on a chip to revoke that specific `(capability, scope)` pair. Click **+ Add** to open a modal:

- **Capability:** `notify` / `fs.pick` / `fs.read` / `fs.write`
- **Scope:** path prefix for `fs.read` / `fs.write`, blank for `notify` and `fs.pick`

Grants take effect on the app's next launch — the bridge preloads permissions when the WebView is created, so a running app keeps its original grant set until restarted.

### Kill switch

Each open app row shows **Kill**, which:

1. Removes the app from `app_windows`, dropping its `Window` and `WebView` — closing the OS window and ending the renderer.
2. Transitions the instance to `Lifecycle::Killed`.
3. Writes `ended_at` to the SQLite session row.

Closed apps show **Launch** instead, which revives them via the same `UserEvent::Launch` path the launcher grid uses.

---

## Capability Model

Every app declares which native capabilities it needs. Grants live in `(app_id, capability, scope)`. Permissions are preloaded into the bridge at window-creation time.

| Capability | Scope | Native implementation |
|---|---|---|
| `fs.pick` | none (user-mediated) | `rfd::FileDialog::pick_files` |
| `fs.read` | path prefix | `std::fs::read_to_string` (canonicalized) |
| `fs.write` | path prefix | `std::fs::write` (canonicalized) |
| `notify` | none | `notify_rust::Notification::show` |

Example seed grants (from `seed_catalog`):

```rust
catalog.grant_permission("notion", "fs.read",  r"C:\Users\Public")?;
catalog.grant_permission("notion", "fs.write", r"C:\Users\Public")?;
catalog.grant_permission("slack", "notify",   "")?;
```

Both the requested path and the granted scope are canonicalized before the prefix check, blocking `..` traversal and symlink escapes. The dashboard's permission editor calls the same `grant_permission` / `revoke_permission_scope` methods the seeder uses.

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

1. **Origin gate** — the IPC event's page host must match the app's launch origin (or a subdomain). Blocks third-party pages during OAuth redirects or malicious navigations.
2. **Capability gate** — the app must have an entry for that capability.
3. **Scope gate** — for path-taking capabilities, the canonicalized requested path must fall within a canonicalized granted scope.

Failure returns a structured error without touching the OS:

```json
{ "ok": false, "error": "path 'C:\\Windows' is outside the granted scope for 'fs.read'", "request_id": "req-1" }
```

Responses travel back through the Tao event loop — `UserEvent::BridgeResponse` carries a script that dispatches a `reeve-response` `CustomEvent` on `window`. This avoids the deadlock that occurs when calling `evaluate_script` from inside the IPC handler during WebView construction.

---

## Popup Handling

Every app window installs a `new_window_req_handler`. The classifier (`classify_popup`) routes each popup through these gates in order:

| Gate | Rule | Example |
|---|---|---|
| Opaque scheme | `about:`, `blob:`, `data:` → Managed | JS-driven `window.open()` |
| Same origin | Host equals or is a subdomain of the app | `app.linear.app` from `linear.app` |
| Same brand | Second-level labels match | `notion.so` ↔ `app.notion.com` |
| Known OAuth | Explicit allow-list of auth hosts | `accounts.google.com`, `login.microsoftonline.com`, `github.com` |
| OAuth query | `redirect_uri=`, `clientId=`, `popupFlowId=`, `callbackType=popup`, etc. | Custom OAuth providers |

- **Managed** → `NewWindowResponse::Allow`. WebView2 opens a real popup with a valid `WindowProxy` sharing the parent's cookie jar. OAuth completes and `window.open()` doesn't return `null`, so sites don't show their "popups blocked" message.
- **SystemBrowser** → `open::that(url)` and `NewWindowResponse::Deny`. External links open in the user's default browser and never load inside the shared `WebContext`.

The query heuristic is case-insensitive and covers both snake_case (`redirect_uri=`) and camelCase (`redirectUri=`) forms, plus Notion/Firebase-style `popupFlowId=` markers.

---

## Project Layout

```
reeve/
├── Cargo.toml
├── Makefile
├── build.rs                # winres: embeds assets/reeve.ico
├── README.md
├── .gitignore
├── assets/
│   └── reeve.ico           # multi-resolution Windows icon
├── migrations/
│   └── 001_init.sql
└── src/
    ├── main.rs             # entry, seed_catalog(), launches the UI
    ├── catalog.rs          # SQLite: apps, permissions, sessions
    ├── environment.rs      # shared WebView2 data directory
    ├── governor.rs         # Job Object, memory queries, process snapshots
    ├── bridge.rs           # capability checks + native dispatch
    ├── instance.rs         # app lifecycle state machine
    └── ui.rs               # launcher, dashboard, IPC, popups, URL install
```

> `migrations/001_init.sql` must exist at the project root. `src/catalog.rs` includes it via `include_str!("../migrations/001_init.sql")`.
>
> `assets/reeve.ico` must exist for `build.rs` to succeed. Convert a PNG with `magick input.png -define icon:auto-resize=256,128,64,48,32,16 assets/reeve.ico`.

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
| 30-app seed catalog, idempotent | ✅ |
| Embedded `.exe` icon + per-app favicons | ✅ |
| Dashboard: live memory, permission editor, kill switch | ✅ |
| PWA manifest ingestion (icon, name from manifest.json) | ⏳ |
| Per-app memory attribution | ⏳ (blocked by WebView2 API) |
| `sessions.peak_mem_bytes` from live Job Object stats | ⏳ |
| Benchmark harness (Electron vs Reeve vs Chrome tabs) | ⏳ |
| macOS target (WKWebView, soft memory limits) | ⏳ |
| Linux target (WebKitGTK, cgroups v2) | ⏳ |

---

## The Benchmark We Owe You

The "80% less RAM" claim will be published with full methodology:

1. Cold-start the same five apps (Linear, Notion, Figma, Slack, Spotify) as **Electron apps**, as **Reeve PWAs**, and as **five Chrome tabs**.
2. Measure peak working set and committed memory per configuration over 10 minutes.
3. Publish raw numbers and the harness.

The dashboard's **Total WebView2 memory** is the number the benchmark will report for the Reeve configuration — same data source, same methodology, so the published claim and the live UI will always agree.

Expectation: Reeve will significantly beat Electron, and will be roughly at parity with Chrome tabs — but with per-app memory caps, a capability bridge, and a managed dashboard that Chrome does not offer. The honest comparison is Electron → Reeve.

---

## Security Model

The bridge is the attack surface. Design principles:

- **Deny by default.** No capability without an explicit grant.
- **Origin-gated IPC.** Pages on the app's declared origin (or subdomain) only.
- **Scope every path.** Both sides canonicalized; symlinks and `..` rejected.
- **User-mediated pick.** `fs.pick` uses the OS dialog — the user chooses the file.
- **Job Object isolation.** Runaway renderers are capped at the kernel level.
- **Kill on close.** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` prevents orphaned renderers.
- **Kill switch.** The dashboard can terminate any app instantly, dropping its WebView and ending its session.

Not a formal security audit. Known limitations to address before production:

- **Subdomain matching is naive** — `host.ends_with(".app.example")` is safe for exact hosts, but a full implementation must consult the Public Suffix List to avoid `evil.co.uk` matching `.co.uk`.
- **No rate limiting on the bridge.** A malicious app could spam `fs.pick` with dialogs.
- **No audit log.** Bridge calls should be persisted for post-incident review.
- **WebView2 renderers are in a private job.** Per-process memory limits apply only to processes Reeve can attach; the job-wide cap on the browser process covers the rest.
- **Kill switch is not a security boundary.** It's a UX tool for stopping misbehaving apps. A malicious app could re-open itself via the launcher before you can kill it — the actual containment is the Job Object cap, not the kill button.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `couldn't read migrations/001_init.sql` | Confirm `migrations/001_init.sql` exists at the project root |
| `couldn't read assets/reeve.ico` | Confirm `assets/reeve.ico` exists; `build.rs` requires it |
| `ICoreWebView2Environment13` not found | Match `webview2-com` version to what `cargo tree -p wry` reports |
| `GetProcessMemoryInfo` not found | Confirm `Win32_System_ProcessStatus` feature is enabled in `Cargo.toml` |
| Popup flashes and vanishes | Confirm `wry = "0.55"` or later — older versions run the handler off-thread |
| Site shows "popups blocked" | The URL was denied. Check the `NewWindowRequested` log and add the host to `OAUTH_HOSTS` or extend `looks_like_oauth` |
| Renderer attachment logs `Access is denied` | Expected — WebView2 sandboxes renderers; the browser-process cap covers the tree |
| Dashboard shows 0 B memory | No apps are open yet — click one in the launcher grid first |
| New seed apps don't appear | Seeding runs every launch via `seed_catalog()`; check the log for `add_app` failures |

---

## Contributing

Reeve is in early development. The most valuable contributions right now:

1. **PWA manifest ingestion.** The search bar currently records only the URL. Fetch `manifest.json`, extract name, icons, theme color, and scope — this is the biggest UX gap left.
2. **Benchmark harness.** A reproducible script for the three-way comparison above.
3. **Public Suffix List matching** for the origin gate.
4. **Toolhelp32 fallback** for WebView2 runtimes older than ~2023.
5. **macOS and Linux targets.** The governance layer is stubbed on non-Windows; porting requires WKWebView + memory-pressure signals on macOS, WebKitGTK + cgroups v2 on Linux.

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

The shared-runtime model was inspired by the observation that modern desktop apps are just web apps with a browser bundled inside. Reeve removes the bundle.

The dashboard's honest aggregate memory metric was a deliberate choice: WebView2 doesn't expose per-controller memory attribution, so rather than fabricate per-app numbers, Reeve shows what the OS actually sees. The savings are real; the dashboard proves it without lying about the granularity.