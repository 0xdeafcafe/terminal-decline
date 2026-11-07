import Slope from './Slope';
import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="wrap open" aria-labelledby="h-open">
      <div className="text">
        <h1 id="h-open"><span>26 MB</span>{' '}<span>at idle.</span></h1>
        <p className="lead">A terminal for macOS. 100 MB through <code>cat</code> in 0.58 s.</p>
      </div>
      <div className="right">
        <div className="ghost-wrap" aria-hidden="true"><pre className="ghost">{'  screen ID [--json] [--scrollback N]\n  resize ID COLSxROWS\n  screenshot ID FILE.png'}</pre></div>
        <TerminalPreview />
        <div className="slope"><Slope /></div>
      </div>
    </section>
  );
}
