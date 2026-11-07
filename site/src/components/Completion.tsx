import TerminalWindow from './TerminalWindow';
import completion from '../data/completion-preview.json';

export default function Completion() {
  return (
    <section className="band" aria-labelledby="h-tab">
      <div className="wrap">
        <div className="section-head">
          <h2 id="h-tab">Tab explains the flags.</h2>
          <p>Completions from zsh, with descriptions from man pages and <code>--help</code>.</p>
        </div>
        <figure className="completion-preview" aria-label="An excerpt of git fetch completions in Decline">
          <TerminalWindow>
            <div className="completion-screen">
              <p className="completion-command"><span className="terminal-prompt" aria-hidden="true">➜  ~ </span><code>{completion.command}</code><span className="terminal-cursor" aria-hidden="true" /></p>
              <ul className="completion-list">
                {completion.rows.map(row => (
                  <li key={row.flag} className={row.flag === completion.selected ? 'selected' : undefined}>
                    <span className="completion-marker" aria-hidden="true">{row.flag === completion.selected ? '▸' : ' '}</span>
                    <code>{row.flag}</code>
                    <span className="completion-description">{row.description}</span>
                  </li>
                ))}
              </ul>
              <div className="completion-hint" aria-hidden="true">⏎ put in · esc close</div>
            </div>
          </TerminalWindow>
        </figure>
      </div>
    </section>
  );
}
