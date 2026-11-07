export default function Agents() {
  return (
    <section className="band field" aria-labelledby="h-ctl">
      <div className="wrap two">
        <div className="copy agent-copy">
          <h2 id="h-ctl">Agent control</h2>
          <p>Agents can run programs in headless terminals and read their screens as text or JSON.</p>
          <p><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/design/agent-api.md">ctl reference</a></p>
        </div>
        <pre className="agent-commands">{'decline ctl spawn --name sh --shell\ndecline ctl send sh --text "make test" --key enter\ndecline ctl wait-for sh --command --timeout 600000\ndecline ctl screen sh --json\ndecline ctl kill sh'}</pre>
      </div>
    </section>
  );
}
