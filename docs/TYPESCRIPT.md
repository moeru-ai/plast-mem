# TypeScript Conventions

These conventions apply to `benchmarks/` and `examples/`.

## Workspace assumptions

- package manager: `pnpm`
- root workspace uses `catalog:` dependencies
- do not use `npm install` inside this repo

## TypeScript config

Use `@moeru/tsconfig` as the base config:

```json
{
  "extends": "@moeru/tsconfig",
  "compilerOptions": {
    "noEmit": true,
    "types": ["node"]
  },
  "include": ["src/**/*.ts"]
}
```

Notes:

- `moduleResolution: "bundler"` comes from the base config
- workspace packages such as `plastmem` export `.ts` sources directly, so keep
  bundler-style import resolution
- if you create a new TS package, add its `tsconfig.json` to the root
  `tsconfig.json` references

## Lint rules that matter in practice

The repo uses `eslint.config.js` with `@moeru/eslint-config`.

Common constraints:

- prefer arrow functions over named `function` declarations
- no top-level side effects except the explicit entrypoint call
- use `exit` from `node:process`, not `process.exit(...)`
- `console.log` is rejected; use `stdout.write(...)` for CLI output
- keep imports sorted
- avoid implicit boolean coercion on nullable values

## CLI entrypoint pattern

Match the style already used in `benchmarks/locomo/src/cli.ts`:

```ts
import { exit, loadEnvFile } from 'node:process'

const main = async (): Promise<void> => {
  try {
    loadEnvFile(...)
  }
  catch {}

  // real work
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((error) => {
  console.error(error)
  exit(1)
})
```

## Useful commands

```bash
pnpm exec eslint benchmarks/locomo/src/export-memories.ts
pnpm -F @plastmem/benchmark-locomo exec tsc --noEmit
pnpm -F @plastmem/haru exec tsc --noEmit
```

## Current packages

- `benchmarks/locomo`
- `benchmarks/longmemeval`
- `examples/haru`
- `packages/plastmem`
