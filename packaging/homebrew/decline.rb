# Template: .github/workflows/release.yml replaces @VERSION@ and @SHA256@ and
# commits the result to Casks/decline.rb in 0xdeafcafe/homebrew-tap.
cask "decline" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/0xdeafcafe/terminal-decline/releases/download/v#{version}/Decline-#{version}-macos-arm64.zip"
  name "Decline"
  desc "GPU-accelerated terminal emulator"
  homepage "https://github.com/0xdeafcafe/terminal-decline"

  # The app updates itself (Sparkle); brew upgrade --greedy also works.
  auto_updates true
  depends_on arch: :arm64
  depends_on macos: ">= :sonoma"

  app "Decline.app"
  binary "#{appdir}/Decline.app/Contents/MacOS/decline"

  caveats <<~EOS
    Dock pins made with `decline pin` load decline from Decline.app, so keep
    it installed while you use pins.
  EOS

  zap trash: [
    "~/.config/decline",
    "~/Library/Logs/decline",
    "~/Library/Preferences/red.forbes.decline.plist",
    "~/Library/Saved Application State/red.forbes.decline.savedState",
  ]
end
