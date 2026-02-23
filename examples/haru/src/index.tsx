import { join } from 'node:path'
import { loadEnvFile } from 'node:process'

import { render } from 'ink'

import { ChatApp } from './chat'
import { workspaceDir } from './utils/workspace-dir'

const main = async () => {
  try {
    loadEnvFile(join(workspaceDir, '.env'))
  }
  catch { }

  // eslint-disable-next-line no-console
  console.clear()

  render(<ChatApp />)
}

// eslint-disable-next-line antfu/no-top-level-await
await main()
