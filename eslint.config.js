import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { defineConfig } from '@moeru/eslint-config'

const rootDir = fileURLToPath(new URL('.', import.meta.url))
const tsconfigPath = resolve(rootDir, 'tsconfig.eslint.json')

export default defineConfig({
  react: true,
  typescript: {
    tsconfigPath,
    parserOptions: {
      // Ensure project-service resolves relative paths against repo root,
      // not VSCode ESLint's sometimes-changing CWD.
      tsconfigRootDir: rootDir,
    },
  },
}).append({
  rules: {
    'toml/padding-line-between-pairs': 'off',
  },
})
