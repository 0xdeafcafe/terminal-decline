export default function Agents() {
  return (
<section className={"band field"} aria-labelledby={"h-ctl"}>
  <div className={"wrap two"}>
    <div className={"copy"} style={{"display": "grid", "gap": "1rem"}}>
      <h2 id={"h-ctl"} style={{"marginBottom": ".5rem"}}>{"Agents get a real terminal, without a window."}</h2>
      <p><code>{"decline ctl"}</code>{" starts a program in a Decline terminal that has no window. An agent, or a test, types into it, waits for text or for the command to finish, and reads the screen back: as text, or as JSON with every cell, its colours, the cursor and the modes. A screenshot is drawn on the GPU with your fonts and theme."}</p>
      <p>{"It talks to the running app, or starts a headless server with no Dock icon that leaves once it is idle. The socket is reachable only by your user, and "}<code>{"agent_api = false"}</code>{" turns the whole thing off. A terminal nothing has asked about for 30 minutes is killed. There is no assistant inside decline. It is a terminal for yours to drive."}</p>
      <p className={"muted"}>{"The details are in "}<a href={"https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/design/agent-api.md"}>{"the ctl design notes"}</a>{"."}</p>
    </div>
    <div className={"cmd"}>
      <pre>{"terminal-decline ctl spawn --name top --size 100x30 -- htop\nterminal-decline ctl wait-for top --text \"Load average\" --timeout 5000\nterminal-decline ctl screen top --json\nterminal-decline ctl screenshot top top.png\nterminal-decline ctl kill top\n\nterminal-decline ctl spawn --name sh --shell\nterminal-decline ctl send sh --text \"make test\" --key enter\nterminal-decline ctl wait-for sh --command --timeout 600000"}</pre>
      <p className={"small muted"}>{"Command examples, not a captured session. Each prints one line of JSON and exits 0 when done, 3 when a wait timed out."}</p>
    </div>
  </div>
</section>
  );
}
