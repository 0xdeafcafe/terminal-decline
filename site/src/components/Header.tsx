import Wordmark from './Wordmark';

export default function Header() {
  return (
    <header className="plate-header">
      <div className="wrap top">
        <a href="/" aria-label="Terminal Decline, home" className="ui"><Wordmark /></a>
        <div className="install">
          <code className="sel" id="install-command">brew install --cask 0xdeafcafe/tap/terminal-decline</code>
          <button id="copy-install" className="ui" type="button" aria-label="Copy install command">Copy</button>
          <span id="copy-status" className="sr-only" role="status" />
        </div>
      </div>
    </header>
  );
}
