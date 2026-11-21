# mclean

An **open-source**, **command-line** macOS cleaner inspired by CleanMyMac.
A terminal UI (TUI) scans your Mac, then offers a menu: smart clean, caches,
developer junk, large files, duplicates, app leftovers, full uninstall, system
health and a disk space explorer.

No Apple Developer account, no code signing, no telemetry: build it, run it.

```
make
./mclean
```

## Install

You need Rust (once):

```sh
brew install rust            # or: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then in the project folder:

```sh
make            # release build, creates ./mclean
./mclean        # interactive UI
make install    # optional: copies it to ~/.local/bin (PREFIX=/usr/local make install)
```

Because the binary is built on your own machine, Gatekeeper does not quarantine it.

### Full Disk Access

Without it, macOS hides part of the caches and containers. mclean detects this and
offers to open the right setting: **System Settings › Privacy & Security › Full Disk Access**,
enable your terminal app (Terminal, iTerm2, Ghostty…), then restart mclean.

## What mclean looks at

| Module | Contents | Default risk |
|---|---|---|
| ✨ Smart clean | Every "safe" item from the other modules, in one go | safe |
| 🧹 System caches & logs | `~/Library/Caches`, logs, crash reports, Trash, Mail attachments, iOS updates | safe |
| 🌐 Browsers | Safari, Chrome, Arc, Brave, Edge, Firefox, Vivaldi caches (never history or passwords) | safe |
| 🔧 Developer junk | Xcode (DerivedData, DeviceSupport, Archives), npm, pnpm, yarn, bun, pip/uv, Cargo, Go, Gradle, Maven, CocoaPods, SwiftPM, Homebrew, JetBrains, VS Code… | safe / review |
| 📦 Project build artifacts | `node_modules`, `target/`, `.venv`, `Pods`, `.build`, `.gradle`… **only when the project file is present** (`package.json`, `Cargo.toml`…) | review (pre-ticked if the project is idle > 90 days) |
| 🐘 Large & old files | Files > 500 MB, with their last-opened date (Spotlight) | review |
| 👯 Duplicates | Size → partial hash → full xxh3 hash; one copy is always kept | review |
| 📥 Forgotten downloads | `.dmg` / `.pkg` older than 14 days, old archives | safe / review |
| 🧩 App leftovers | Preferences, containers, Application Support of apps whose vendor has **no installed app at all** | review |
| 📤 Uninstall | The app + its preferences, caches, containers, launch agents… | manual |
| 🩺 Health | Disk space, FileVault, firewall, Gatekeeper, SIP, updates, Time Machine, APFS snapshots, swap, memory, battery, Spotlight, broken or unsigned launch agents, corrupted preferences, broken links, Homebrew, Docker, Xcode simulators | — |
| 📊 Disk space | Size-sorted explorer, folder by folder (ncdu / Space Lens style) | — |

### What is the smart clean?

It gathers every item marked **safe** across all categories (caches, logs, installers
left in Downloads, the Trash…), i.e. things that macOS or the apps recreate, or that
can be downloaded again. "Review" items (your projects, large files, duplicates, app
leftovers…) are never included. If you untick something inside a category, it is left
out of the smart clean too. Opening it shows the full list (with the category icon
of each item), where you can expand, tick and untick before cleaning; a confirmation
screen always comes last.

### What is this folder?

Each line comes with a plain-English description ("What is it"), e.g.
`com.apple.mediaanalysisd` → Photos face/object analysis, `*.ShipIt` → an app's
auto-updater downloads, `safebrowsing` → the browser's phishing-site list. Unknown
bundle ids are matched against your installed apps. Descriptions live in
[`src/describe.rs`](src/describe.rs): additions welcome.

### Select manually, then delete selected

Prefer to start from nothing? In the menu, **Select manually** (`m`) unticks
everything in every category. Open the categories, tick only what you want (Space,
Tab to pick single files), come back and choose **Delete selected** (`d`): everything
ticked, in every category, is cleaned in one go after a confirmation screen.

### Picking individual files

In a category, press **Tab** (or **→**) on a line to expand it: every file or folder it contains
gets its own checkbox (e.g. pick only some of the logs in "User logs", or only some
DerivedData projects). **Tab** again (or **←**) folds it. A partially ticked line shows `[~]`;
only the ticked sub-items are cleaned.

## Keys

| Key | Action |
|---|---|
| `↑ ↓` / `j k` | move |
| `Enter` | open, confirm, clean |
| `Tab` | expand / fold an item to pick its files one by one |
| `→` / `l` | expand an item |
| `←` / `h` | collapse, or go back |
| `Esc` / `q` | go back one step (`q` quits from the menu) |
| `Space` | tick / untick |
| `a` | tick / untick everything |
| `o` | show in Finder |
| `p` | switch Trash ⇄ permanent deletion |
| `s` `m` `d` | smart clean, select manually, delete selected (from the menu) |
| `e` `u` | explore disk space, uninstall (from the menu) |
| `1`–`9` | jump to a menu entry |
| `r` | scan again |
| `?` | help |
| `q` | quit |

## Without the UI

```sh
./mclean scan            # text report
./mclean scan --json     # JSON report (for scripts)
./mclean clean --dry-run # what the smart clean would do
./mclean clean --yes     # non-interactive smart clean (scheduled task)
./mclean rules           # active rules
```

## Safety

- **Trash by default**: everything is recoverable until you empty the Trash. Permanent deletion (`p`) is shown in red.
- **Central safety net** (`actions::check_safe`): refuses anything outside your home folder (except `/Applications/*.app` for uninstalls), top-level folders (`~/Documents`, `~/Library`…), the keychain, `~/.ssh`, iCloud Drive, Mail, Messages, photo libraries, `/System`, `/usr`, `/Library`…
- Symlinks are never followed.
- Cleaning is refused while the related app is running (Chrome, Xcode…).
- Operations that need `sudo` (firewall, system agents…) are never run by mclean itself: it shows the command, pauses the UI and runs it in your terminal, where you type your password.
- Journal of every deletion: `~/Library/Logs/mclean/journal.tsv`.

## Your own rules

Create `~/.config/mclean/rules.toml` (evaluated before the defaults; the same `id` overrides a default rule):

```toml
[[rule]]
id = "my-tool"
name = "My tool's cache"
category = "dev"            # system | browsers | dev
risk = "safe"               # safe | review
paths = ["~/.my-tool/cache"]
explanation = "Downloaded again automatically."
```

Every field is documented at the top of [`rules/default.toml`](rules/default.toml).
Rule contributions are welcome: it is the easiest way to improve mclean.

## Development

```sh
make test                             # unit tests
make lint                             # rustfmt + clippy
sh scripts/fake-home.sh /tmp/fakehome # fake "dirty" home folder
HOME=/tmp/fakehome ./mclean           # try it without touching your real files
```

```
src/
├── main.rs          CLI (clap): TUI, scan, clean, rules
├── model.rs         Finding, SubItem, Category, Risk, HealthItem, Fix
├── rules.rs         declarative rule engine (TOML)
├── apps.rs          installed apps, related files, leftovers
├── actions.rs       safety net, Trash, deletion, journal
├── util.rs          disk usage, glob, commands with timeout
├── scan/
│   ├── mod.rs       runs the 4 scans in parallel
│   ├── home.rs      home folder walk (projects, large files, downloads)
│   ├── dupes.rs     duplicates
│   └── health.rs    health checks
└── tui/             ratatui UI (menu, categories, health, uninstall, disk space)
```

## Credits and licenses

mclean is released under **GPL-3.0-or-later**. Original code; ideas and file locations studied in these projects:

| Project | License | What inspired mclean |
|---|---|---|
| [Mole](https://github.com/tw93/Mole) | GPL-3.0 | cache location lists, safety checks for project artifacts |
| [czkawka](https://github.com/qarmin/czkawka) | MIT (core and CLI) | multi-pass duplicate detection |
| [dua-cli](https://github.com/Byron/dua-cli) | MIT | TUI disk explorer |
| [gdu](https://github.com/dundee/gdu) | MIT | disk explorer |
| [dust](https://github.com/bootandy/dust) | Apache-2.0 | disk usage computation |
| [mac-cleanup-py](https://github.com/mac-cleanup/mac-cleanup-py) | Apache-2.0 | developer cache list |
| [KnockKnock](https://github.com/objective-see/KnockKnock) | GPL-3.0 | launch agent inspection |
| [Pearcleaner](https://github.com/alienator88/Pearcleaner) | Apache-2.0 + **Commons Clause** (not OSI open source) | concept only, no code reused |

GPL-3.0 lets you reuse code from Mole or KnockKnock (GPL) and from the MIT/Apache projects.
Pearcleaner is not compatible: don't copy code from it.

## Ideas for later

- APFS clone detection for duplicates (actual space freed)
- Similar photos (perceptual hash)
- XProtect YARA signatures for a protection module
- Scheduled scans through a LaunchAgent (`mclean clean --yes`)
- Login items list (`sfltool dumpbtm`)
