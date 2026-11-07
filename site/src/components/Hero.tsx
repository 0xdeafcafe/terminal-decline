import Slope from './Slope';
import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="wrap open" aria-labelledby="h-open">
      <div className="text">
        <h1 id="h-open"><span>26 MB</span>{' '}<span>at idle.</span></h1>
        <p className="lead">Terminal Decline is a terminal for macOS, written in Rust and drawn with Metal. A 100 MB file goes through <code>cat</code> in 0.58 s.</p>
        <p className="small muted">Measured on an Apple M1 Max. The comparison is from 2 November 2026.</p>
        <p className="links"><a href="#install">Install with Homebrew</a> <a href="https://github.com/0xdeafcafe/terminal-decline">Read the source</a></p>
      </div>
      <div className="right">
        <div className="ghost-wrap" aria-hidden="true"><pre className="ghost">{'  screen ID [--json] [--scrollback N]\n  resize ID COLSxROWS\n  screenshot ID FILE.png'}</pre></div>
        <TerminalPreview />
        <div className="slope"><Slope /></div>
      </div>
    </section>
  );
}
