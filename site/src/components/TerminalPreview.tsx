import screen from '../data/terminal-preview.json';

/** Real ctl cells, grouped into equal-style runs. No terminal emulator shipped. */
export default function TerminalPreview() {
  return (
    <figure className="terminal-preview">
      <div className="terminal-window">
        <div className="terminal-titlebar" aria-hidden="true">
          <span className="window-controls"><i /><i /><i /></span>
          <span>Terminal Decline</span>
        </div>
      <pre className="terminal-screen" aria-label="Comparison recorded on 2 November 2026">
        {screen.rows.map((row, y) => (
          <span className="terminal-row" key={y}>
            {row.map((run, x) => (
              <span key={x} style={{ color: run.color, fontWeight: run.bold ? 700 : 400 }}>{run.text}</span>
            ))}
            {y < screen.rows.length - 1 ? '\n' : ''}
          </span>
        ))}
      </pre>
      </div>
      <figcaption className="small muted">The recorded results, displayed in Decline. <a href="#measured">Test conditions</a>.</figcaption>
    </figure>
  );
}
