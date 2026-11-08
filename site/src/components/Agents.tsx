import CommandLines from './CommandLines';
import Completion from './Completion';
import { agentCommands } from '../data/commands';

export default function Agents() {
  return (
    <>
      <section id="agents" aria-labelledby="h-ctl">
        <div className="duo flip">
          <div>
            <h2 id="h-ctl">Let your coding agent use the terminal</h2>
            <p className="prose">With <code>decline ctl</code>, an agent can run a command and wait for it to finish, then read what’s on screen. It can do this without opening a window.</p>
            <p className="small prose"><a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/design/agent-api.md">How to use decline ctl</a></p>
          </div>
          <div><CommandLines lines={agentCommands} className="agent-term" /></div>
        </div>
      </section>
      <section id="completion" aria-labelledby="h-completion">
        <h2 id="h-completion">See what a flag does</h2>
        <p className="prose">Press Tab to see matching flags and subcommands, with explanations from the command’s man page or <code>--help</code>. You can also turn on suggestions as you type.</p>
        <div className="comp-term narrow"><Completion /></div>
      </section>
    </>
  );
}
