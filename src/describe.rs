//! Plain-English descriptions of cache folders, bundle ids and file names, so
//! the user knows what something is before deleting it.

use crate::apps;
use crate::util;
use std::collections::HashMap;
use std::sync::OnceLock;

/// (pattern, description). Patterns use `*` / `?` and are matched
/// case-insensitively against a folder or file name. First match wins.
const KNOWN: &[(&str, &str)] = &[
    // ── macOS services ──
    (
        "com.apple.mediaanalysisd",
        "Photos analysis: faces, objects, scenes and Live Text found in your photo library. If deleted, macOS redoes the analysis in the background (CPU/battery for a while).",
    ),
    (
        "com.apple.photoanalysisd",
        "Photos Memories and People analysis. Rebuilt in the background if deleted.",
    ),
    (
        "com.apple.helpd",
        "Index of the built-in macOS Help books. Rebuilt the next time you open Help.",
    ),
    (
        "com.apple.parsecd",
        "Spotlight & Siri Suggestions results (web suggestions in search). Re-downloaded on demand.",
    ),
    (
        "com.apple.CloudTelemetry*",
        "iCloud diagnostics/telemetry data waiting to be sent to Apple.",
    ),
    (
        "com.apple.TV",
        "Apple TV app: artwork, posters and metadata.",
    ),
    (
        "com.apple.Music",
        "Music app: artwork and streaming cache (not your downloaded songs).",
    ),
    ("com.apple.podcasts", "Podcasts app cache (artwork, feeds)."),
    ("com.apple.news*", "Apple News articles and images."),
    ("com.apple.Maps", "Apple Maps tiles and place data."),
    (
        "GeoServices",
        "Maps tiles and location lookups used by Maps, Weather, Photos… Re-downloaded as needed.",
    ),
    (
        "com.apple.AppleMediaServices",
        "App Store / Apple Music / Apple TV shared store data (artwork, catalog).",
    ),
    (
        "com.apple.appstoreagent",
        "App Store background download agent cache.",
    ),
    (
        "com.apple.StoreKit*",
        "In-app purchase / App Store sheet cache.",
    ),
    (
        "com.apple.amsengagementd",
        "App Store / Apple services promotions and engagement cache.",
    ),
    (
        "com.apple.SetupAssistant",
        "Setup Assistant data (shown after macOS updates).",
    ),
    (
        "com.apple.wallpaper*",
        "Wallpaper cache: Aerial videos and dynamic wallpaper images. Aerials are re-downloaded if you use them.",
    ),
    (
        "com.apple.python",
        "Bytecode cache of the Python bundled with macOS / Xcode tools.",
    ),
    (
        "com.apple.nsurlsessiond",
        "Background downloads made by macOS and apps (often leftovers of finished downloads).",
    ),
    (
        "com.apple.QuickLook*",
        "Quick Look thumbnails shown by Finder.",
    ),
    (
        "com.apple.iconservices*",
        "App and file icon cache. Rebuilt automatically (icons may flicker briefly).",
    ),
    ("com.apple.Safari*", "Safari data cache."),
    ("com.apple.dt.*", "Xcode / developer tools cache."),
    ("com.apple.iBooks*", "Apple Books cache (covers, store)."),
    (
        "com.apple.BKAgentService",
        "Apple Books background service cache.",
    ),
    ("com.apple.gamed", "Game Center cache."),
    ("com.apple.passd", "Wallet / Apple Pay passes cache."),
    ("com.apple.siri*", "Siri cache."),
    (
        "com.apple.Spotlight*",
        "Spotlight interface cache (not the search index).",
    ),
    (
        "com.apple.textunderstandingd",
        "On-device text analysis (detected dates, addresses… in your text).",
    ),
    ("com.apple.findmy*", "Find My cache."),
    ("com.apple.FaceTime", "FaceTime cache."),
    ("com.apple.Photos*", "Photos app cache (not your library)."),
    ("com.apple.preview*", "Preview app cache."),
    ("com.apple.WeatherKit*", "Weather data cache."),
    (
        "SharedImageCache",
        "Shared image thumbnails cache used by system apps.",
    ),
    (
        "com.apple.*",
        "Built-in macOS component cache. Recreated by macOS when needed.",
    ),
    // ── App updaters ──
    (
        "*.ShipIt",
        "Auto-updater (Squirrel/ShipIt) downloads of an app update, usually already installed.",
    ),
    (
        "*updater*",
        "Downloaded app updates, usually already installed.",
    ),
    (
        "*.update",
        "Downloaded app updates, usually already installed.",
    ),
    (
        "org.sparkle-project.Sparkle",
        "Sparkle auto-updater downloads, usually already installed.",
    ),
    (
        "Mozilla",
        "Firefox auto-updater downloads (Mozilla/updates), usually already installed.",
    ),
    // ── Third-party apps ──
    (
        "Waves Audio",
        "Waves Central: downloaded plugin installers and update packages.",
    ),
    (
        "Logic",
        "Logic Pro caches (waveform overviews, sound library index).",
    ),
    (
        "com.spotify.client",
        "Spotify streaming cache (songs you played recently, not downloads).",
    ),
    (
        "com.microsoft.*",
        "Microsoft app cache (Office, Teams, Edge…).",
    ),
    ("com.google.*", "Google app cache."),
    ("Google", "Google apps caches (Chrome, Android Studio…)."),
    ("com.docker.docker", "Docker Desktop cache."),
    ("com.tinyspeck.slackmacgap*", "Slack cache."),
    ("com.hnc.Discord", "Discord cache."),
    ("JetBrains", "JetBrains IDE caches."),
    ("Homebrew", "Homebrew downloaded packages."),
    ("Yarn", "Yarn package cache."),
    ("pip", "Python pip package cache."),
    ("node-gyp", "Node.js native module build headers."),
    ("deno", "Deno module and compile cache."),
    ("claude", "Claude Code CLI cache."),
    ("uv", "uv (Python) package cache."),
    ("huggingface", "Downloaded AI models (Hugging Face)."),
    (
        "ms-playwright",
        "Browsers downloaded by Playwright for testing.",
    ),
    ("go-build", "Go compiler build cache."),
    ("CocoaPods", "CocoaPods pod cache."),
    ("electron", "Electron binaries downloaded by npm packages."),
    // ── Inside browser caches ──
    (
        "safebrowsing",
        "Safe Browsing database (list of phishing/malware sites). Re-downloaded automatically.",
    ),
    ("cache2", "Firefox web cache."),
    ("startupCache", "Firefox startup cache."),
    (
        "Code Cache",
        "Compiled JavaScript cache of the browser/app.",
    ),
    ("GPUCache", "GPU shader cache."),
    ("Service Worker", "Website offline data (service workers)."),
    (
        "CacheStorage",
        "Website offline storage (service worker caches).",
    ),
    ("Cache", "Web cache."),
    ("Default", "Default browser profile."),
    ("Profile *", "Additional browser profile."),
];

fn installed_names() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        apps::installed(true)
            .into_iter()
            .map(|a| (a.bundle_id.to_lowercase(), a.name))
            .collect()
    })
}

/// Describes a folder or file name, e.g. `com.apple.mediaanalysisd`.
pub fn describe(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let key = lower
        .trim_end_matches(".plist")
        .trim_end_matches(".savedstate");
    // An installed app with this exact bundle id (or its helper) tells us best.
    let apps = installed_names();
    if let Some(app) = apps.get(key) {
        if let Some((_, d)) = KNOWN.iter().find(|(p, _)| {
            !p.starts_with('*') && util::wildcard(&p.to_lowercase(), key) && *p != "com.apple.*"
        }) {
            return Some(format!("{app}: {d}"));
        }
        return Some(format!("Cache of the {app} app, recreated when needed."));
    }
    if let Some((_, d)) = KNOWN
        .iter()
        .find(|(p, _)| util::wildcard(&p.to_lowercase(), key))
    {
        return Some((*d).to_string());
    }
    // Helper of an installed app: com.vendor.app.helper → "App".
    if let Some(app) = apps
        .iter()
        .filter(|(id, _)| key.starts_with(&format!("{id}.")))
        .max_by_key(|(id, _)| id.len())
        .map(|(_, app)| app)
    {
        return Some(format!(
            "Belongs to {app} (helper or component), recreated when needed."
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_names() {
        assert!(
            describe("com.apple.mediaanalysisd")
                .unwrap()
                .contains("Photos")
        );
        assert!(
            describe("org.whispersystems.signal-desktop.ShipIt")
                .unwrap()
                .contains("updater")
        );
        assert!(
            describe("ledger-live-desktop-updater")
                .unwrap()
                .contains("updates")
        );
        assert!(
            describe("com.apple.SomethingNew")
                .unwrap()
                .contains("macOS")
        );
        assert!(describe("safebrowsing").unwrap().contains("phishing"));
        assert!(describe("zzz-unknown").is_none());
    }
}
