import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="plate hero" aria-labelledby="h-open">
      <div className="wrap">
        <h1 id="h-open">A peppy terminal for macOS.</h1>
        <p className="lead">A fast, lightweight terminal. Crisp text, no bloat, and built to stay lean.</p>
        <TerminalPreview />
      </div>
    </section>
  );
}
