import type { ReactNode } from 'react';

export default function TerminalWindow({ children }: { children: ReactNode }) {
  return (
    <div className="terminal-window">
      <div className="terminal-titlebar" aria-hidden="true">
        <span className="window-controls"><i /><i /><i /></span>
        <span>Terminal Decline</span>
      </div>
      {children}
    </div>
  );
}
