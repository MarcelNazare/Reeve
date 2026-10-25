# Reeve

**One engine. Many apps. Governed.**

![Reeve Launcher](assets/reeve-product-snapshot.png)

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

- ✅ **Launcher UI:** Modern dark theme, URL-based install, and a real-time search/filter bar.
- ✅ **App Management:** Right-click (or click the `⋮` icon) to uninstall apps or clear their local data.
- ✅ **Developer SDK:** Automatically injects a Promise-based `window.reeve` API into every app.
- ✅ **Security:** Public Suffix List (PSL) origin matching and 500ms rate limiting on native calls.
- ✅ **Self-Healing Icons:** Fetches `manifest.json` for real app names. Local icons are Base64-encoded to guarantee rendering. Native windows inherit the app icon.
- ✅ **Database Deduplication:** Safely cleans up duplicate apps on startup, cascading deletes to foreign keys.
- ✅ **Shared Runtime:** One WebView2 browser process across all apps.
- ✅ **Job Object Governor:** Kernel-enforced per-process and job-wide memory caps.
- ✅ **Capability Bridge:** `fs.pick`, `fs.read`, `fs.write`, `notify`.
- ✅ **Smart Popup Handling:** OAuth and same-brand popups open as managed windows; external links route to the system browser.
- ✅ **Managed Dashboard:** Live memory, editable permissions, kill switch, and peak memory tracking.
- ✅ **Benchmark Harness:** Repeatable cold-start and steady-state memory measurement.

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

## Building & Running

Reeve is a workspace with two binaries: `reeve` (launcher) and `benchmark` (harness).

```bash
git clone https://github.com/MarcelNazare/Reeve.git
cd Reeve
cargo check
cargo run                      # launches the launcher
cargo run --bin benchmark -- --help  # launches the benchmark harness
```

Or with the bundled `Makefile`:

```bash
make          # build
make run      # build + run launcher with debug logging
make release  # optimized build (LTO, stripped)
```

**Make on Windows:** run `make` from **Git Bash** or **MSYS2**, not `cmd.exe`. The `run` target sets `RUST_LOG` with POSIX shell syntax.

Data directory:
```
%APPDATA%\Reeve\Reeve\data\
├── catalog.db      # apps, permissions, sessions
├── icons\          # cached PWA icons
└── webview\        # shared WebView2 user-data folder
```

---

## Adding Apps

Paste a URL into the top search bar and press **Add**. Reeve immediately records the app, then spawns a background thread that:
1. Fetches the page HTML and locates `<link rel="manifest">`.
2. Parses the `manifest.json`.
3. Extracts the real app name.
4. Downloads the highest-resolution icon and caches it locally in `data/icons/`.
5. Updates the grid automatically when finished. (Falls back to Google favicons if no manifest exists).

You can also seed apps programmatically in `src/main.rs`. Seeding is idempotent and will not create duplicates.

---

## Developer SDK

Reeve injects a prelude script into every launched app, exposing a Promise-based SDK on the `window` object. You no longer need to manage `postMessage` or `request_id` routing.

```javascript
// Read a file (requires fs.read permission)
try {
  const { content } = await window.reeve.fs.read("C:\\Users\\Public\\notes.txt");
  console.log(content);
} catch (err) {
  console.error("Access denied:", err.message);
}

// Send a native notification (requires notify permission)
await window.reeve.notify("Build Complete", "Deployment finished in 3.2s.");

// Open a native file picker (requires fs.pick permission)
const { paths } = await window.reeve.fs.pick();
```

Every call passes through three gates:
1. **Origin gate**: IPC page host must match the app's launch origin (using PSL eTLD+1 matching).
2. **Capability gate**: App must have an explicit grant in the database.
3. **Scope gate**: Canonicalized requested paths must fall within a canonicalized granted scope.

---

## Dashboard

Click **Dashboard** in the launcher header. A second window opens, sharing the same WebView2 environment.

- **Live Memory:** Sum of working set across the entire tree.
- **Peak job memory:** Highest committed memory ever seen by the Windows Job Object.
- **Per-app Memory:** Attributed by matching renderer frames to the app's `start_url`.
- **Permission Editor:** Grant or revoke `notify`, `fs.pick`, `fs.read`, or `fs.write` per app.
- **Kill Switch:** Instantly terminate a running app's renderer and end its SQLite session.

---

## Benchmark

The harness in `src/bin/benchmark.rs` runs configurations in isolation.

### The WebView2 Cold-Start Bottleneck

Initial benchmarks showed a massive `t_stable` delay of **41.1 seconds** for Reeve, compared to **9.2 seconds** for Chrome tabs. 

By isolating the WebView2 initialization process, we discovered the bottleneck is **GPU process and DirectComposition initialization**. WebView2 blocks the host application while it negotiates with your graphics driver and builds its initial DirectComposition surface.

When running with software rendering (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--disable-gpu"`), cold-start memory stabilization drops from **41.1s to 3.3s** — over 10x faster than Chrome.

**The tradeoff:** Software rendering makes heavy canvas apps (Figma, Google Maps) unusable. Reeve now ships with `--disable-features=CalculateNativeWinOcclusion` to mitigate Windows window-throttling, keeping hardware acceleration intact.

### The Snapshot

<details>
<summary>Click to view the raw benchmark output</summary>

```markdown
# Reeve benchmark

Test apps: https://linear.app, https://notion.so, https://figma.com, https://app.slack.com/client, https://open.spotify.com

## Cold start

| Scenario | t_stable (proxy) | t_ready (exact) |
|---|---|---|
| reeve | 41100 ms | 12460 ms |
| chrome-tabs | 9200 ms | — |

## Steady state

| Scenario | Peak memory | Steady memory | Peak processes |
|---|---|---|---|
| reeve | 2575.5 MB | 2165.5 MB | 18 |
| chrome-tabs | 2212.2 MB | 2195.3 MB | 24 |
```
</details>

### What this tells us

1. **Steady-state memory is at parity.** Reeve (2165 MB) and Chrome (2195 MB) use essentially the same amount of RAM for these five heavy apps. The "80% less RAM" claim holds against Electron, but **not** against Chrome tabs.
2. **Process count is lower.** Reeve achieves parity with 18 processes vs Chrome's 24.
3. **GPU initialization is the cold-start tax.** WebView2's synchronous GPU setup adds ~30 seconds to the memory stabilization time on this hardware.

---

## Project Layout

```
reeve/
├── Cargo.toml
├── Makefile
├── build.rs                # winres: embeds assets/reeve.ico
├── README.md
├── LICENSE                 # MIT
├── assets/
│   ├── reeve.ico           # multi-resolution Windows icon
│   └── reeve-product-snapshot.png
├── migrations/
│   └── 001_init.sql
└── src/
    ├── main.rs             # entry point for the launcher (default-run)
    ├── catalog.rs          # SQLite: apps, permissions, sessions
    ├── environment.rs      # shared WebView2 data directory
    ├── governor.rs         # Job Object, memory queries, process snapshots
    ├── bridge.rs           # capability checks + native dispatch
    ├── instance.rs         # app lifecycle state machine
    ├── manifest.rs         # PWA manifest ingestion (fetch, parse, download icon)
    ├── ui.rs               # launcher, dashboard, IPC, popups, URL install, search
    └── bin/
        └── benchmark.rs    # entry point for the benchmark harness
```

---

## Roadmap

| Milestone | Status |
|---|---|
| Shared environment, one browser process | ✅ |
| Job Object memory caps, renderer attachment | ✅ |
| Capability-gated bridge (v1 capabilities) | ✅ |
| PSL Origin gate + Rate Limiting | ✅ |
| Promise-based Developer SDK | ✅ |
| App Management (Uninstall, Clear Data) | ✅ |
| Benchmark harness (Electron vs Reeve vs Chrome tabs) | ✅ |
| Cold-start benchmark | ✅ |
| **Partitioned Cookies** (isolating app data) | ⏳ |
| **Native Context Menus** (Win32 popup for right-click) | ⏳ |
| macOS target (WKWebView, soft memory limits) | ⏳ |
| Linux target (WebKitGTK, cgroups v2) | ⏳ |

---

## Security Model

The bridge is the attack surface. Design principles:

- **Deny by default.** No capability without an explicit grant.
- **PSL Origin matching.** Uses the Public Suffix List (`psl` crate) to compare eTLD+1 domains. This prevents `evil.co.uk` from matching an app on `co.uk`.
- **Rate limiting.** The bridge rejects more than 1 call per 500ms to prevent malicious apps from spamming native dialogs.
- **Scope every path.** Both sides canonicalized; symlinks and `..` rejected.
- **User-mediated pick.** `fs.pick` uses the OS dialog — the user chooses the file.
- **Job Object isolation.** Runaway renderers are capped at the kernel level.

Known limitations:
- **Shared cookie jar.** All apps share the same WebView2 profile. Logging out of Slack logs you out of Notion if they share an OAuth provider. Partitioned cookies are on the roadmap.
- **Kill switch is not a security boundary.** It's a UX tool. The actual containment is the Job Object cap.

---

## License

MIT. See [LICENSE](LICENSE).

---

## Acknowledgements

Reeve stands on the shoulders of:

- **[wry](https://github.com/tauri-apps/wry)** and **[tao](https://github.com/tauri-apps/tao)** — WebView and windowing
- **[windows](https://github.com/microsoft/windows-rs)** — Win32 API bindings
- **[webview2-com](https://github.com/wravery/webview2-rs)** — WebView2 COM bindings
- **[psl](https://github.com/addr-rs/psl)** — Public Suffix List parsing
- **[reqwest](https://github.com/seanmonstar/reqwest)** and **[scraper](https://github.com/causal-agent/scraper)** — PWA manifest ingestion
- **[image](https://github.com/image-rs/image)** — Native window icon decoding

The shared-runtime model was inspired by the observation that modern desktop apps are just web apps with a browser bundled inside. Reeve removes the bundle.