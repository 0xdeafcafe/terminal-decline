import TerminalWindow from './TerminalWindow';
import CommandLines from './CommandLines';
import Completion from './Completion';
import screen from '../data/terminal-preview.json';
import { previewCommands } from '../data/commands';

export default function TerminalPreview() {
  return (
    <TerminalWindow>
      <div className="panes">
        <div className="pane">
          <pre className="term sel" aria-label="Comparison recorded on 2 November 2026">
            {screen.rows.map((row, y) => (
              <span key={y}>
                {row.map((run, x) => <span key={x} style={{ color: run.color, fontWeight: run.bold ? 700 : 400 }}>{run.text}</span>)}
                {y < screen.rows.length - 1 ? '\n' : ''}
              </span>
            ))}
          </pre>
        </div>
        <div className="right">
          <div className="pane" aria-label="Example commands"><CommandLines lines={previewCommands} /></div>
          <div className="pane"><Completion /></div>
        </div>
      </div>
    </TerminalWindow>
  );
}
