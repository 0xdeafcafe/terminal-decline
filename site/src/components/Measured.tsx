import comparison from '../data/comparison.json';
import type { ReactNode } from 'react';

type MetricKey = 'idle' | 'tabs' | 'scrollback' | 'cat' | 'cpu' | 'idleCpu';
const metrics: { key: MetricKey; label: ReactNode; unit: string; decimals: number }[] = [
  { key: 'idle', label: 'Idle memory', unit: ' MB', decimals: 0 },
  { key: 'tabs', label: 'Four tabs', unit: ' MB', decimals: 0 },
  { key: 'scrollback', label: 'After 10k lines', unit: ' MB', decimals: 0 },
  { key: 'cat', label: <><code>cat</code> 100 MB</>, unit: ' s', decimals: 2 },
  { key: 'cpu', label: <>CPU for <code>cat</code></>, unit: ' s', decimals: 2 },
  { key: 'idleCpu', label: 'Idle CPU', unit: '%', decimals: 2 },
];

export default function Measured() {
  return (
    <section id="numbers" aria-labelledby="h-num">
      <div className="nums-head">
        <h2 id="h-num">Against Ghostty and kitty</h2>
        <p>2 November 2026. M1 Max, macOS 27.0. Medians of five runs. Lower is better.</p>
      </div>
      <table>
        <caption>Bars are relative within each row.</caption>
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
        <p className="muted">Memory is physical footprint across all processes. CPU for <code>cat</code> includes WindowServer.</p>
        <p>Menlo 13, 100×30 cells, 10k lines of scrollback. One window. Runs interleaved at system load 3–6. Ghostty 1.4.0, kitty 0.49.2. <a href={comparison.source}>Builds, script and full results</a>.</p>
        <p>On 30 October, Ghostty finished the <code>cat</code> in 0.53 s against Decline’s 0.61 s. Neither run compared input latency. <a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/comparison-2026-10-30.md">30 October results</a>.</p>
      </div>
    </section>
  );
}
