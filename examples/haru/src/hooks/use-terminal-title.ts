import { stdout } from 'node:process'

import { useEffect } from 'react'

export const useTerminalTitle = (title: string) => useEffect(() => {
  stdout.write(`\x1B]0;${title}\x07`)

  return () => {
    stdout.write('\u001B]0;\u0007')
  }
}, [title])
