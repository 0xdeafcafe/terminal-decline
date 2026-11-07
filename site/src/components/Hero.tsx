import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="plate hero" aria-labelledby="h-open">
      <div className="wrap">
        <h1 id="h-open">A peppy terminal for macOS.</h1>
        <p className="lead">26 MB idle. 0.58 s to <code>cat</code> 100 MB.</p>
        <p className="lead-sub">Measured against Ghostty and kitty on an M1 Max, 2 November 2026, medians of five runs.</p>
        <TerminalPreview />
      </div>
    </section>
  );
}
