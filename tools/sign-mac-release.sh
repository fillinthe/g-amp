#!/bin/sh
# Signs a release's Mac downloads with the Developer ID certificate, has Apple
# notarize them, and puts them back on the release, so they open with a
# double-click. Runs on the Mac that holds the certificate, after the Release
# workflow has made the draft (the certificate never goes to GitHub).
#
#   tools/sign-mac-release.sh v0.3.17
#
# Needs: the signing keychain (GAMP_KEYCHAIN, its password in the file next
# to it named keychain-password), and notarytool credentials saved once with
#   xcrun notarytool store-credentials gamp --apple-id <id> --team-id <team>
# NOTARIZE=0 signs only (for a try-out).
set -e
OUT=$PWD
cd "$(dirname "$0")/.."
TAG=${1:?usage: tools/sign-mac-release.sh vX.Y.Z}
KC=${GAMP_KEYCHAIN:-$HOME/.gamp-signing/gamp-release.keychain-db}
PROFILE=${GAMP_NOTARY_PROFILE:-gamp}
security unlock-keychain -p "$(cat "$(dirname "$KC")/keychain-password")" "$KC"
# The Developer ID identity in that keychain, by fingerprint (another with the same name may be elsewhere).
ID=$(security find-identity -v -p codesigning "$KC" | awk '/Developer ID Application/ { print $2; exit }')
[ -n "$ID" ] || { echo "no Developer ID Application identity in $KC"; exit 1; }

WORK=$(mktemp -d)
trap 'hdiutil detach -quiet "$WORK/mnt" 2>/dev/null; rm -rf "$WORK"' EXIT
gh release download "$TAG" -p '*macOS*.dmg' -D "$WORK"
for dmg in "$WORK"/*.dmg; do
    name=$(basename "$dmg")
    echo "== $name"
    # Sign the app inside, in place (the dmg's layout stays as the workflow made it).
    hdiutil convert -quiet "$dmg" -format UDRW -o "$WORK/rw.dmg"
    hdiutil resize -quiet -size 300m "$WORK/rw.dmg"
    hdiutil attach -quiet -nobrowse -mountpoint "$WORK/mnt" "$WORK/rw.dmg"
    codesign --force --deep --options runtime --timestamp --keychain "$KC" --sign "$ID" "$WORK/mnt/Gamp.app"
    codesign --verify --deep --strict "$WORK/mnt/Gamp.app"
    hdiutil detach -quiet "$WORK/mnt"
    rm "$dmg"
    hdiutil convert -quiet "$WORK/rw.dmg" -format UDZO -o "$dmg"
    rm "$WORK/rw.dmg"
    codesign --force --timestamp --keychain "$KC" --sign "$ID" "$dmg"
    if [ "${NOTARIZE:-1}" = 1 ]; then
        xcrun notarytool submit "$dmg" --keychain-profile "$PROFILE" --wait
        xcrun stapler staple "$dmg"
        spctl -a -t open --context context:primary-signature -v "$dmg"
    fi
done
if [ "${NOTARIZE:-1}" = 1 ]; then
    gh release upload "$TAG" "$WORK"/*.dmg --clobber
    echo "signed, notarized and uploaded: $TAG"
else
    mv "$WORK"/*.dmg "$OUT"/
    echo "signed only (not uploaded), in $OUT"
fi
