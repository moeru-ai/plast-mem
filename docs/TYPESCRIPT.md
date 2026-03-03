# TypeScript Conventions (examples/ and benchmarks/)

## ESLint Config (`@antfu/eslint-config` + `@moeru/eslint-config`)

Key rules enforced — violating these causes lint errors:

- **`prefer-arrow/prefer-arrow-functions`**: No `function foo()` declarations. Always `const foo = () =>`
- **`@masknet/no-top-level`**: No side-effect calls at module top level. Move into functions; use `// eslint-disable-next-line @masknet/no-top-level` for unavoidable entry-point invocations (e.g. `main().catch(...)`)
- **`node/prefer-global/process`**: Always `import process from 'node:process'` explicitly
- **`ts/strict-boolean-expressions`**: No implicit boolean coercion. `if (!str)` on `string | undefined` → `if (str == null || str.length === 0)`; `if (num)` on `number` → `if (num != null && num > 0)`
- **`ts/no-use-before-define` (variables: true)**: `const` arrow functions don't hoist. Define helpers before their callers
- **`@masknet/prefer-timer-id`**: `setTimeout`/`setInterval` return values must be assigned: `const timer = setTimeout(...); void timer`
- **`no-console`**: Only `console.warn`/`console.error` allowed in library code. Use `process.stdout.write(str + '\n')` for output
- **`depend/ban-dependencies`**: `dotenv` is banned — use `process.loadEnvFile()` (Node.js v20.12+) inside a `try/catch`
- **`perfectionist/sort-imports`** with `newlinesBetween: 1`: Import groups in order: `type` imports → `node:` builtins → external packages → local. One blank line between groups

## TypeScript Config (`@moeru/tsconfig`)

- `moduleResolution: "bundler"` — required for importing workspace packages that export `.ts` source directly (like `plastmem`)
- `allowImportingTsExtensions: true` + `noEmit: true` — bundler mode assumption; compilation via `tsx` at runtime
- Import paths: **no `.js` extensions** (bundler mode resolves without them)
- All new `tsconfig.json` files in `examples/` or `benchmarks/` should `extend: "@moeru/tsconfig"` and be added to the root `tsconfig.json` references

## AI / LLM

- Use `@xsai/generate-text` (`generateText`) — not `openai` SDK directly. `openai` has a `zod@^3` peer dep conflict with workspace's zod v4
- Env vars: `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_CHAT_MODEL`; read via `process.env` after `process.loadEnvFile()`

## Patterns

```typescript
// sleep utility (reuse across files)
const sleep = (ms: number): Promise<void> =>
  new Promise<void>((resolve) => { const t = setTimeout(resolve, ms); void t })

// load env at start of main(), not top-level
const main = async () => {
  try { process.loadEnvFile(resolve(__dirname, '../.env')) } catch {}
  // ...
}

// TOCTOU: don't existsSync then read — just try/catch
const loadJson = (path: string) => {
  try { return JSON.parse(readFileSync(path, 'utf-8')) }
  catch { return {} }
}

// reuse __dirname instead of calling fileURLToPath twice
const __dirname = dirname(fileURLToPath(import.meta.url))
// then: resolve(__dirname, '../.env')  — not fileURLToPath(import.meta.url) again
```
