import { stdout } from 'node:process';
import { useEffect } from 'react';

export function useTerminalTitle(title: string) {
  useEffect(() => {
    stdout.write(`\x1b]0;${title}\x07`);

    return () => {
      stdout.write(`\x1b]0;\x07`);
    };
  }, [title])
}
