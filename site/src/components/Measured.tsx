import comparison from '../data/comparison.json';
import earlierComparison from '../data/comparison-2026-10-30.json';

interface Measurements {
  name: string;
  idle: number;
  cat: number;
}
const metrics = [
  { key: 'idle', label: 'Idle memory', unit: ' MB' },
  { key: 'cat', label: '100 MB output', unit: ' s' },
] as const;

function ComparisonCharts({ date, terminals }: { date: string; terminals: Measurements[] }) {
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
      <p className="small prose">{earlierComparison.date}, M1 Max. Lower is better.</p>
      <ComparisonCharts date={earlierComparison.date} terminals={earlierComparison.terminals} />
      <p className="small prose">Warp ran with two windows.</p>
      <details className="benchmark-method">
        <summary>Newer results and method</summary>
        <p className="prose">{comparison.date}, M1 Max.</p>
        <ComparisonCharts date={comparison.date} terminals={comparison.terminals} />
        <div className="notes">
          <p>Memory is physical footprint. Output time is for <code>cat</code> of a 100 MB text file. Neither run measured input latency.</p>
          <p>2 November: medians of five runs. <a href={comparison.source}>Full results and method</a>.</p>
          <p>30 October: medians of four runs per terminal, three for Terminal.app. <a href={earlierComparison.source}>Full results and method</a>.</p>
        </div>
      </details>
    </section>
  );
}
