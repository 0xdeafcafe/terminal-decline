import TerminalPreview from './TerminalPreview';

export default function Hero() {
  return (
    <section className="wrap open" aria-labelledby="h-open">
      <div className="text">
        <h1 id="h-open">A terminal<br />for macOS.</h1>
        <p className="lead">26 MB at idle. 100 MB through <code>cat</code> in 0.58 s.</p>
        <p className="hero-features"><a href="#h-pin">Pin commands to the Dock.</a><br /><a href="#h-ctl">Give agents their own terminals.</a></p>
      </div>
      <TerminalPreview />
    </section>
  );
}
