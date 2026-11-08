import Slope from './Slope';

export default function Install() {
  return (
    <section id="install" aria-labelledby="h-inst" className="closing">
      <div className="slope ui"><Slope /></div>
      <div>
        <h2 id="h-inst">Install Decline</h2>
        <p className="prose">On an Apple Silicon Mac running macOS 14 Sonoma or later, install with Homebrew:</p>
        <code className="cmdline sel">brew install --cask 0xdeafcafe/tap/terminal-decline</code>
        <p className="small prose">Then open Terminal Decline from Applications. The <code>decline</code> command is installed too.</p>
        <ul className="links">
          <li><a href="https://github.com/0xdeafcafe/terminal-decline#install">Build from source</a></li>
          <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/DESIGN.md">Technical notes</a></li>
          <li><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/CHANGELOG.md">Changelog</a></li>
        </ul>
      </div>
    </section>
  );
}
