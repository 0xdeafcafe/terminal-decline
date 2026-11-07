export const pinCommand = 'decline pin rush --dock';

// --key enter submits the text; a literal backslash-n does not.
export const agentCommands = [
  'decline ctl spawn --name sh --shell',
  'decline ctl send sh --text "make test" --key enter',
  'decline ctl wait-for sh --command --timeout 600000',
  'decline ctl screen sh --json',
  'decline ctl kill sh',
];
export const previewCommands = [pinCommand, agentCommands[0], agentCommands[1], agentCommands[3]];
