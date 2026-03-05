import { defineConfig } from '@moeru/eslint-config'

export default defineConfig({
  pnpm: { sort: true },
  react: true,
}).append({
  rules: {
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
