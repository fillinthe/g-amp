#!/bin/sh
# Builds Gamp in release and installs it as /Applications/Gamp.app,
# replacing what's there. The bundle is laid out by hand (the same files
# cargo-packager puts in the dmg), ad-hoc signed so macOS runs it.
# GAMP_ALSO_INSTALL=user@host also copies the app to that Mac's
# /Applications over ssh (key auth).
set -e
cd "$(dirname "$0")/.."
PATH="$HOME/.cargo/bin:$PATH"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=/Applications/Gamp.app
STAGE=target/Gamp.app

cargo build --release -p gamp

rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp target/release/gamp "$STAGE/Contents/MacOS/gamp"
cp assets/logo/gamp.icns "$STAGE/Contents/Resources/gamp.icns"
cat > "$STAGE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>English</string>
	<key>CFBundleDisplayName</key><string>Gamp</string>
	<key>CFBundleExecutable</key><string>gamp</string>
	<key>CFBundleIconFile</key><string>gamp.icns</string>
	<key>CFBundleIdentifier</key><string>co.fillinthe.gamp</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>Gamp</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$VERSION</string>
	<key>CFBundleVersion</key><string>$(date +%Y%m%d.%H%M%S)</string>
	<key>LSApplicationCategoryType</key><string>public.app-category.music</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>mp3</string><string>flac</string><string>m4a</string><string>aac</string>
				<string>ogg</string><string>oga</string><string>wav</string><string>aif</string>
				<string>aiff</string><string>m3u</string><string>m3u8</string><string>pls</string>
			</array>
			<key>CFBundleTypeName</key><string>Audio</string>
			<key>CFBundleTypeRole</key><string>Viewer</string>
		</dict>
		<dict>
			<key>CFBundleTypeExtensions</key><array><string>wsz</string></array>
			<key>CFBundleTypeName</key><string>Winamp skin</string>
			<key>CFBundleTypeRole</key><string>Viewer</string>
		</dict>
	</array>
	<key>NSHighResolutionCapable</key><true/>
	<key>NSHumanReadableCopyright</key><string>© 2026 Fill in the Blank, LLC</string>
	<key>NSLocalNetworkUsageDescription</key><string>Gamp plays music from the servers and NAS boxes on your network.</string>
	<key>NSBonjourServices</key><array><string>_ssdp._udp</string></array>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$STAGE" >/dev/null 2>&1 || true

RUNNING=$(pgrep -f "$APP/Contents/MacOS/gamp" || true)
if [ -n "$RUNNING" ]; then
  echo "Quitting the running Gamp…"
  osascript -e 'tell application "Gamp" to quit' >/dev/null 2>&1 || kill $RUNNING || true
  sleep 1
fi
rm -rf "$APP" /Applications/G-Amp.app
cp -R "$STAGE" "$APP"
echo "Installed Gamp $VERSION at $APP"

if [ -n "$GAMP_ALSO_INSTALL" ]; then
  SSH="ssh -o BatchMode=yes -o ConnectTimeout=8"
  $SSH "$GAMP_ALSO_INSTALL" 'pgrep -f Gamp.app/Contents/MacOS >/dev/null && { osascript -e "tell application \"Gamp\" to quit"; sleep 1; }; rm -rf /tmp/Gamp.app'
  rsync -a -e "$SSH" "$APP" "$GAMP_ALSO_INSTALL:/tmp/"
  $SSH "$GAMP_ALSO_INSTALL" 'rm -rf /Applications/Gamp.app && mv /tmp/Gamp.app /Applications/ && xattr -cr /Applications/Gamp.app'
  echo "Installed Gamp $VERSION on $GAMP_ALSO_INSTALL"
fi
