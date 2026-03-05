# TypeScript Conventions (examples/ and benchmarks/)

## ESLint Config (`@antfu/eslint-config` + `@moeru/eslint-config`)

Key rules enforced — violating these causes lint errors:

- **`prefer-arrow/prefer-arrow-functions`**: No `function foo()` declarations. Always `const foo = () =>`
- **`@masknet/no-top-level`**: No side-effect calls at module top level. Move into functions; use `// eslint-disable-next-line @masknet/no-top-level` for unavoidable entry-point invocations (e.g. `main().catch(...)`)
- **`node/prefer-global/process`**: Always `import { ... } from 'node:process'` explicitly
- **`ts/strict-boolean-expressions`**: No implicit boolean coercion. `if (!str)` on `string | undefined` → `if (str == null || str.length === 0)`; `if (num)` on `number` → `if (num != null && num > 0)`
- **`ts/no-use-before-define` (variables: true)**: `const` arrow functions don't hoist. Define helpers before their callers
- **`@masknet/prefer-timer-id`**: `setTimeout`/`setInterval` return values must be assigned: `const timer = setTimeout(...); void timer`
- **`no-console`**: Only `console.warn`/`console.error` allowed in library code. Use `stdout.write(str + '\n')` for output
- **`depend/ban-dependencies`**: `dotenv` is banned — use `loadEnvFile()` (Node.js v20.12+) inside a `try/catch`
- **`perfectionist/sort-imports`** with `newlinesBetween: 1`: Import groups in order: `type` imports → `node:` builtins → external packages → local. One blank line between groups

## TypeScript Config (`@moeru/tsconfig`)

- `moduleResolution: "bundler"` — required for importing workspace packages that export `.ts` source directly (like `plastmem`)
- `allowImportingTsExtensions: true` + `noEmit: true` — bundler mode assumption; compilation via `tsx` at runtime
- Import paths: **no `.js` extensions** (bundler mode resolves without them)
- All new `tsconfig.json` files in `examples/` or `benchmarks/` should `extend: "@moeru/tsconfig"` and be added to the root `tsconfig.json` references

## AI / LLM

- Use `@xsai/generate-text` (`generateText`) — not `openai` SDK directly. `openai` has a `zod@^3` peer dep conflict with workspace's zod v4
- Env vars: `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_CHAT_MODEL`; read via `env` after `loadEnvFile()`
