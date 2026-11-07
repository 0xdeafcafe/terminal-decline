import type { ReactNode } from 'react';

export default function TerminalWindow({ children }: { children: ReactNode }) {
  return (
    <div className="win" role="group" aria-label="Terminal Decline preview">
      <div className="bar ui" aria-hidden="true">
        <span className="lights"><i /><i /><i /></span>
        <span className="title">Terminal Decline</span>
      </div>
      <nav className="tabs ui" aria-label="Page sections">
        <a href="#numbers">Footprint</a><a href="#pins">Pins</a><a href="#agents">Agents</a><a href="#install">Install</a>
      </nav>
      {children}
    </div>
  );
}
