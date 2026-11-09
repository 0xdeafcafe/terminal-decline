import comparison from '../data/comparison.json';

interface Measurements {
  name: string;
  idle: number;
  cat: number;
}
const metrics = [
  { key: 'idle', label: 'Idle memory', unit: ' MB' },
  { key: 'cat', label: '100 MB output', unit: ' s' },
] as const;

function ComparisonChart({ date, terminals }: { date: string; terminals: Measurements[] }) {
  return (
    <div className="comparison-charts" data-comparison={date}>
      {metrics.map(metric => {
        const max = Math.max(...terminals.map(terminal => terminal[metric.key]));
        return <figure key={metric.key}>
          <figcaption>{metric.label}</figcaption>
          <ul className="benchmark-bars">
            {terminals.map(terminal => {
              const value = terminal[metric.key];
              return <li key={terminal.name} className={terminal.name === 'Decline' ? 'is-decline' : undefined}>
                <span className="benchmark-name">{terminal.name}</span>
                <span className="benchmark-track" aria-hidden="true"><span style={{ width: `${value / max * 100}%` }} /></span>
                <span className="benchmark-value">{metric.key === 'cat' ? value.toFixed(2) : value}{metric.unit}</span>
              </li>;
            })}
          </ul>
        </figure>;
      })}
    </div>
  );
}

export default function Measured() {
  return (
    <section id="numbers" aria-labelledby="h-num">
      <h2 id="h-num">How it compares</h2>
      <p className="small prose">{comparison.date}, on {comparison.machine}, {comparison.system}. Medians of {comparison.runs} rounds. Lower is better.</p>
      <ComparisonChart date={comparison.date} terminals={comparison.terminals} />
      <p className="small prose">Warp ran with two windows.</p>
      <details className="benchmark-method">
        <summary>Method</summary>
        <div className="notes">
          <p>Memory is physical footprint, summed over each terminal’s processes. Output time is for <code>cat</code> of a 100 MB text file. Each terminal ran the same script on the same clean runner, taken in turns. kitty could not run there: it needs OpenGL 3.3, which the runner’s virtual GPU lacks. <a href={comparison.source}>Full results and method</a>, with CPU, input latency and four tabs.</p>
          <p>Earlier runs on an M1 Max, which include kitty: <a href="https://github.com/0xdeafcafe/terminal-decline#benchmarks">2 November</a> and <a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/comparison-2026-10-30.md">30 October</a>.</p>
        </div>
      </details>
    </section>
  );
}
