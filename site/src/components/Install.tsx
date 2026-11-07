import Slope from './Slope';

export default function Install() {
  return (
    <section id="install" aria-labelledby="h-inst" className="closing">
      <div className="slope ui"><Slope /></div>
      <div>
        <h2 id="h-inst">Install</h2>
        <p className="prose">Releases are for Apple Silicon and need macOS 14 Sonoma or later.</p>
        <code className="cmdline sel">brew install --cask 0xdeafcafe/tap/terminal-decline</code>
        <p className="small prose">Installs Terminal Decline.app and the command as <code>terminal-decline</code> and <code>decline</code>.</p>
        <ul className="links">
          <li><a href="https://github.com/0xdeafcafe/terminal-decline#install">Build from source</a></li>
          <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/DESIGN.md">Technical notes</a></li>
          <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/CHANGELOG.md">Changelog</a></li>
        </ul>
      </div>
    </section>
  );
}
