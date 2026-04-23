import { defineConfig } from '@moeru/eslint-config'

export default defineConfig({
  pnpm: { sort: true },
  react: true,
}).append({
  rules: {
    'sonarjs/publicly-writable-directories': 'off',
    'toml/padding-line-between-pairs': 'off',
  },
}).append({
  ignores: [
    'crates', // rust
    'src', // rust
    '**/*.toml', // rust
    '**/*.gen.ts', // generated
    'packages/plastmem', // generated
  ],
})
