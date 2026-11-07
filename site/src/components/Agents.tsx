import CommandLines from './CommandLines';
import Completion from './Completion';
import { agentCommands } from '../data/commands';

export default function Agents() {
  return (
    <section id="agents" aria-labelledby="h-ctl">
      <div className="duo flip">
        <div>
          <h2 id="h-ctl">Terminals for agents</h2>
          <p className="prose">Run programs in headless terminals. Type into them, wait on them and read the screen back as text or JSON, with cells, colours and cursor.</p>
          <p className="small prose"><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/design/agent-api.md">ctl reference</a></p>
        </div>
        <div><CommandLines lines={agentCommands} className="agent-term" /></div>
      </div>
      <p className="prose also">Tab completion explains flags and subcommands from man pages and <code>--help</code>. Completion as you type, output colouring and copy on select are opt-in.</p>
      <div className="comp-term narrow"><Completion /></div>
    </section>
  );
}
