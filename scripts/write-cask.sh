#!/usr/bin/env bash
# Print the Homebrew cask for a release, with the disk image's real checksum.
#
#   scripts/write-cask.sh <version> <Perch-x.y.z-macos.dmg> > Casks/perch.rb
#
# `zap` names only what Perch itself creates. The Claude Code directory is never
# touched, by the app or by uninstalling it.
set -euo pipefail

VERSION=${1:?usage: write-cask.sh <version> <dmg>}
DMG=${2:?usage: write-cask.sh <version> <dmg>}
SHA=$(shasum -a 256 "$DMG" | cut -d' ' -f1)

cat <<EOF
cask "perch" do
  version "$VERSION"
  sha256 "$SHA"

  url "https://github.com/amirdaraee/perch/releases/download/v#{version}/Perch-#{version}-macos.dmg"
  name "Perch"
  desc "Menu bar dashboard for Claude Code"
  homepage "https://amirdaraee.com/perch/"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :sequoia

  app "Perch.app"

  uninstall quit: "dev.perch.app"

  zap trash: [
    "~/Library/Application Support/Perch",
    "~/Library/Preferences/dev.perch.app.plist",
  ]
end
EOF
