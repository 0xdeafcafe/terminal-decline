import comparison from '../data/comparison.json';
import type { ReactNode } from 'react';

type MetricKey = 'idle' | 'tabs' | 'scrollback' | 'cat' | 'cpu' | 'idleCpu';
const metrics: { key: MetricKey; label: ReactNode; unit: string; decimals: number }[] = [
  { key: 'idle', label: 'Idle memory', unit: ' MB', decimals: 0 },
  { key: 'tabs', label: 'Memory, four tabs', unit: ' MB', decimals: 0 },
  { key: 'scrollback', label: 'Memory, 10k lines', unit: ' MB', decimals: 0 },
  { key: 'cat', label: 'Display 100 MB', unit: ' s', decimals: 2 },
  { key: 'cpu', label: 'CPU time, same file', unit: ' s', decimals: 2 },
  { key: 'idleCpu', label: 'Idle CPU', unit: '%', decimals: 2 },
];

export default function Measured() {
  return (
    <section id="numbers" aria-labelledby="h-num">
      <div className="nums-head">
        <h2 id="h-num">How it compares</h2>
        <p>In this run, Decline used less than half the memory of Ghostty at idle. We also timed how long each terminal took to display a 100 MB text file.</p>
      </div>
      <table>
        <caption>Tested on an M1 Max, 2 November 2026. Lower is better. Compare bars within each row.</caption>
        <thead><tr><th scope="col">Measure</th>{comparison.terminals.map(terminal => <th scope="col" key={terminal.name}>{terminal.name}</th>)}</tr></thead>
        <tbody>{metrics.map(metric => {
          const values = comparison.terminals.map(terminal => terminal[metric.key]);
          const min = Math.min(...values), max = Math.max(...values);
          return <tr key={metric.key}>
            <th scope="row">{metric.label}</th>
            {comparison.terminals.map(terminal => <td key={terminal.name} className={terminal[metric.key] === min ? 'best' : undefined}>
              <span className="n">{terminal[metric.key].toFixed(metric.decimals)}<span className="u">{metric.unit}</span></span>
              <span className="b" style={{ width: `${terminal[metric.key] / max * 100}%` }} aria-hidden="true" />
            </td>)}
          </tr>;
        })}</tbody>
      </table>
      <div className="notes">
        <p className="muted">The file test uses <code>cat</code> to send text to the terminal. It measures output speed, not how quickly a keystroke appears. CPU time includes WindowServer. Memory is the physical footprint of all terminal processes.</p>
        <p>Each result is the median of five runs on macOS 27.0, taking turns between terminals. One window, Menlo 13, 100×30 cells and 10k lines of scrollback. System load 3–6. Ghostty 1.4.0, kitty 0.49.2. <a href={comparison.source}>Builds, script and full results</a>.</p>
        <p>Ghostty was quicker in the earlier test on 30 October: 0.53 s to display the file, against Decline’s 0.61 s. Neither test compared input latency. <a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/comparison-2026-10-30.md">30 October results</a>.</p>
      </div>
    </section>
  );
}
