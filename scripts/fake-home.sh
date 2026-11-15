#!/bin/sh
# Creates a fake "dirty" home folder to test mclean without touching the real one.
# Usage: scripts/fake-home.sh /tmp/fakehome && HOME=/tmp/fakehome ./mclean
set -eu
H=${1:?usage: $0 <folder>}
rm -rf "$H"; mkdir -p "$H"; cd "$H"
blob() { mkdir -p "$(dirname "$1")"; head -c "$2" /dev/urandom > "$1"; }
old()  { touch -d "$2 days ago" "$1" 2>/dev/null || touch -t "$(date -v-"$2"d +%Y%m%d%H%M)" "$1"; }

blob "Library/Caches/com.spotify.client/data" 30000000
blob "Library/Caches/Google/Chrome/Default/Cache/f" 20000000
blob "Library/Caches/Google/AndroidStudio2024.1/idx" 12000000
blob "Library/Caches/Homebrew/downloads/node.tar.gz" 15000000
blob "Library/Developer/Xcode/DerivedData/MyApp-abc/Build/x" 50000000
blob "Library/Developer/Xcode/iOS DeviceSupport/16.4 (20E247)/Symbols/x" 40000000
old  "Library/Developer/Xcode/iOS DeviceSupport/16.4 (20E247)" 200
blob "Library/Logs/SomeApp/app.log" 3000000; old "Library/Logs/SomeApp" 10
blob ".npm/_cacache/content/a" 12000000
blob ".cache/pip/http/a" 8000000
blob ".Trash/old.zip" 5000000

mkdir -p dev/oldproj dev/rustproj dev/notaproject/target
echo '{}' > dev/oldproj/package.json
blob dev/oldproj/node_modules/lodash/x 30000000
old dev/oldproj/package.json 120
echo '[package]' > dev/rustproj/Cargo.toml
blob dev/rustproj/target/debug/x 25000000
blob dev/notaproject/target/important.bin 25000000

blob Downloads/Installer.dmg 8000000; old Downloads/Installer.dmg 30
blob Downloads/film.mkv 600000000
blob Documents/photo.jpg 3000000
cp Documents/photo.jpg "Downloads/photo copy.jpg"
cp Documents/photo.jpg "Desktop/photo.jpg" 2>/dev/null || { mkdir -p Desktop; cp Documents/photo.jpg Desktop/photo.jpg; }

blob "Library/Application Support/com.oldvendor.gone/db" 2000000
mkdir -p Library/Preferences Library/LaunchAgents
echo '<?xml version="1.0"?><plist version="1.0"><dict/></plist>' > Library/Preferences/com.oldvendor.gone.plist
echo 'pas un plist {{{' > Library/Preferences/com.broken.app.plist
cat > Library/LaunchAgents/com.oldvendor.agent.plist <<P
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>Label</key><string>com.oldvendor.agent</string>
<key>ProgramArguments</key><array><string>/Applications/Gone.app/Contents/MacOS/agent</string></array></dict></plist>
P
mkdir -p Applications/TestApp.app/Contents
cat > Applications/TestApp.app/Contents/Info.plist <<P
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.test.app</string>
<key>CFBundleExecutable</key><string>TestApp</string></dict></plist>
P
blob Applications/TestApp.app/Contents/MacOS/TestApp 4000000
blob Library/Caches/com.test.app/c 6000000
cp Library/Preferences/com.oldvendor.gone.plist Library/Preferences/com.test.app.plist
echo "Fake home folder ready: $H"
