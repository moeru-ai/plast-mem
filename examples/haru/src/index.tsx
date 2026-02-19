import { join } from 'node:path'
import { cwd, loadEnvFile } from 'node:process'

import { findWorkspaceDir } from '@pnpm/find-workspace-dir'
import { render } from 'ink'

import { ChatApp } from './chat'

const main = async () => {
  const workspace = await findWorkspaceDir(cwd())

  try {
    loadEnvFile(join(workspace!, '.env'))
  } catch { }

  console.clear()

  render(<ChatApp />)
}

// if (import.meta.main) {
//   await main()
// }
main()
