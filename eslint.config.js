import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { defineConfig } from '@moeru/eslint-config'

const rootDir = fileURLToPath(new URL('.', import.meta.url))
const tsconfigPath = resolve(rootDir, 'tsconfig.eslint.json')

export default defineConfig({
  pnpm: { sort: true },
  react: true,
  typescript: {
    parserOptions: {
      // Ensure project-service resolves relative paths against repo root,
      // not VSCode ESLint's sometimes-changing CWD.
      tsconfigRootDir: rootDir,
    },
    tsconfigPath,
  },
}).append({
  rules: {
    'toml/padding-line-between-pairs': 'off',
  },
}).append({
  ignores: ['**/*.gen.ts'],
})
