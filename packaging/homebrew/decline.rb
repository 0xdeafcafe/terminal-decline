# Template: .github/workflows/release.yml replaces @VERSION@ and @SHA256@ and
# commits the result to Casks/decline.rb in 0xdeafcafe/homebrew-tap.
cask "decline" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/0xdeafcafe/terminal-decline/releases/download/v#{version}/Decline-#{version}-macos-universal.zip"
  name "Decline"
  desc "GPU-accelerated terminal emulator"
  homepage "https://github.com/0xdeafcafe/terminal-decline"

  depends_on macos: ">= :sonoma"

  app "Decline.app"
  binary "#{appdir}/Decline.app/Contents/MacOS/decline"

  zap trash: [
    "~/Library/Preferences/red.forbes.decline.plist",
    "~/Library/Saved Application State/red.forbes.decline.savedState",
  ]
end
