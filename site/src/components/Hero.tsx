import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="plate hero" aria-labelledby="h-open">
      <div className="wrap">
        <h1 id="h-open">A peppy terminal for macOS.</h1>
        <p className="lead">Decline is a fast, lightweight terminal with native Mac tabs and split panes.</p>
        <TerminalPreview />
      </div>
    </section>
  );
}
