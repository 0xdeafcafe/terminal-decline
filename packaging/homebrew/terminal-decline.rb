# Template: .github/workflows/release.yml replaces @VERSION@ and @SHA256@ and
# commits the result to Casks/terminal-decline.rb in 0xdeafcafe/homebrew-tap
# (with a cask_renames.json entry from the old cask name, decline).
cask "terminal-decline" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/0xdeafcafe/terminal-decline/releases/download/v#{version}/Terminal-Decline-#{version}-macos-arm64.zip"
  name "Terminal Decline"
  desc "GPU-accelerated terminal emulator"
  homepage "https://github.com/0xdeafcafe/terminal-decline"

  # The app updates itself (Sparkle); brew upgrade --greedy also works.
  auto_updates true
  depends_on arch: :arm64
  depends_on macos: ">= :sonoma"

  app "Terminal Decline.app"
  binary "#{appdir}/Terminal Decline.app/Contents/MacOS/terminal-decline"
  binary "#{appdir}/Terminal Decline.app/Contents/MacOS/terminal-decline", target: "decline"

  caveats <<~EOS
    Dock pins made with `terminal-decline pin` load Terminal Decline from
    Terminal Decline.app, so keep it installed while you use pins. Pins made
    by earlier versions (copies of the whole app, including those from
    Decline.app) keep working as they are; `terminal-decline pin --repair`
    rebuilds them as pins that load the installed app.
  EOS

  zap trash: [
    "~/.config/terminal-decline",
    "~/Library/Caches/terminal-decline",
    "~/Library/Logs/terminal-decline",
    "~/Library/Preferences/red.forbes.terminal-decline.plist",
    "~/Library/Saved Application State/red.forbes.terminal-decline.savedState",
    # From before the rename.
    "~/.config/decline",
    "~/Library/Caches/decline",
    "~/Library/Logs/decline",
    "~/Library/Preferences/red.forbes.decline.plist",
    "~/Library/Saved Application State/red.forbes.decline.savedState",
  ]
end
