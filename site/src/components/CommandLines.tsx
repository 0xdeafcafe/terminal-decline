import { Fragment } from 'react';

export default function CommandLines({ lines, className = '' }: { lines: string[]; className?: string }) {
  return (
    <pre className={`term sel ${className}`}>
      {lines.map((line, index) => (
        <Fragment key={line}>
          {line.split(/("[^"\n]*"|--[\w-]+|\bdecline\b)/g).map((part, i) => (
            <span key={i} className={part === 'decline' ? 'g' : part.startsWith('--') ? 'bl' : part.startsWith('"') ? 'y' : undefined}>{part}</span>
          ))}
          {index < lines.length - 1 ? '\n' : ''}
        </Fragment>
      ))}
    </pre>
  );
}
