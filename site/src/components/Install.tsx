export default function Install() {
  return (
    <section className="band install" id="install" aria-labelledby="h-install">
      <span className="corner br" aria-hidden="true">┘</span>
      <div className="wrap section-head">
        <div>
          <h2 id="h-install">Install</h2>
          <pre className="big install-command">brew install --cask 0xdeafcafe/tap/terminal-decline</pre>
        </div>
        <div className="copy">
          <p>Apple Silicon. macOS 14 Sonoma or later.</p>
          <p>Homebrew installs the app and the <code>decline</code> command.</p>
          <ul className="links-list">
            <li><a href="https://github.com/0xdeafcafe/terminal-decline#install">Build from source</a></li>
            <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/DESIGN.md">Technical notes</a></li>
            <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/CHANGELOG.md">Changelog</a></li>
          </ul>
        </div>
      </div>
    </section>
  );
}
