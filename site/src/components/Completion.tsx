import completion from '../data/completion-preview.json';

export default function Completion() {
  return (
    <div className="term sel" role="group" aria-label="git fetch completion preview">
      <span className="line">{completion.command}</span>
      <div className="cmp">
        {completion.rows.map(row => (
          <div key={row.flag} className={`row${row.flag === completion.selected ? ' on' : ''}`}>
            <span className="bl">{row.flag}</span><span className="d">{row.description}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
