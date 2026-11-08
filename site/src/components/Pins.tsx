import { pinCommand } from '../data/commands';

const pins = [
  { name: 'rush', file: 'rush', size: 256 }, { name: 'htop', file: 'htop', size: 256 },
  { name: 'Claude', file: 'claude', size: 256 }, { name: 'Logs', file: 'logs', size: 256 },
  { name: '32 px', file: 'rush-32', size: 32 }, { name: '16 px', file: 'rush-16', size: 16 },
];
export default function Pins() {
  return (
    <section id="pins" aria-labelledby="h-pin">
      <div className="duo">
        <div>
          <h2 id="h-pin">Pin a command to the Dock</h2>
          <p className="prose">Open your favourite command-line app from the Dock. It gets its own windows and settings, just like a Mac app.</p>
          <code className="cmdline sel">{pinCommand}</code>
          <p className="small prose">This example needs <a href="https://github.com/0xdeafcafe/rush">rush</a> installed. <a href="https://github.com/0xdeafcafe/terminal-decline#pin-a-program-to-the-dock">Pin options</a>.</p>
        </div>
        <div>
          <div className="pinrow">{pins.map(pin => <figure key={pin.file}>
            <img src={`/images/${pin.file}.webp`} width={pin.size} height={pin.size} className={pin.size === 32 ? 'sm' : pin.size === 16 ? 'xs' : undefined} alt={pin.size < 100 ? `Rush pin at ${pin.size} pixels` : `Generated ${pin.name} pin`} loading="lazy" decoding="async" />
            <figcaption>{pin.name}</figcaption>
          </figure>)}</div>
          <p className="small prose">Example icons made by Decline. It uses the program’s own icon when one is available.</p>
        </div>
      </div>
    </section>
  );
}
