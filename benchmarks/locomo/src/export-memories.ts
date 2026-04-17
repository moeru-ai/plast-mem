/**
 * Export all episodic and semantic memories for a conversation to the results directory.
 * Usage: tsx src/export-memories.ts <conversation_id> <out_dir> [base_url]
 */
import { writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { argv, env, exit, loadEnvFile, stdout } from 'node:process'
import { fileURLToPath } from 'node:url'

import { recentMemoryRaw, retrieveMemoryRaw } from 'plastmem'

const __dirname = dirname(fileURLToPath(import.meta.url))
const WORKSPACE_ROOT = resolve(__dirname, '../../../')

// Broad queries to sweep semantic memories across different topics
const SEMANTIC_SWEEP_QUERIES = [
  'health fitness exercise gym workout weight',
  'travel vacation trip Canada Banff Rocky Mountains',
  'relationship marriage partner wife love',
  'family children son daughter parents',
  'hobbies painting art drawing classes',
  'work job career stress',
  'food diet nutrition Weight Watchers',
  'car vehicle Prius driving',
  'time date when year month',
  'friends social events party',
]

interface RetrievedSemanticMemories {
  semantic?: Array<{ id: string }>
}

const logLine = (message: string): void => {
  stdout.write(`${message}\n`)
}

const loadWorkspaceEnv = (): void => {
  try {
    loadEnvFile(resolve(WORKSPACE_ROOT, '.env'))
  }
  catch { }
}

const getRequiredArg = (value: string | undefined, usage: string): string => {
  if (value == null || value.length === 0) {
    console.error(usage)
    exit(1)
  }

  return value
}

const getCliConfig = (): {
  baseUrl: string
  conversationId: string
  outDir: string
} => {
  const usage = 'Usage: tsx src/export-memories.ts <conversation_id> <out_dir> [base_url]'
  const conversationId = getRequiredArg(argv[2], usage)
  const outDir = getRequiredArg(argv[3], usage)
  const baseUrl = (argv[4] ?? env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(/\/$/, '')

  return {
    baseUrl,
    conversationId,
    outDir,
  }
}

const exportEpisodicMemories = async (
  conversationId: string,
  baseUrl: string,
): Promise<unknown[]> => {
  logLine('Fetching episodic memories (recent_memory/raw, limit=100)...')
  const res = await recentMemoryRaw({
    baseUrl,
    body: {
      conversation_id: conversationId,
      limit: 100,
    },
    throwOnError: true,
  })
  return (res.data as unknown[]) ?? []
}

const exportSemanticMemories = async (
  conversationId: string,
  baseUrl: string,
): Promise<unknown[]> => {
  logLine(`Sweeping semantic memories with ${SEMANTIC_SWEEP_QUERIES.length} queries...`)
  const seen = new Set<string>()
  const all: unknown[] = []

  for (const query of SEMANTIC_SWEEP_QUERIES) {
    const res = await retrieveMemoryRaw({
      baseUrl,
      body: {
        conversation_id: conversationId,
        episodic_limit: 1,
        query,
        semantic_limit: 100,
      },
      throwOnError: true,
    })
    const data = res.data as null | RetrievedSemanticMemories
    for (const item of data?.semantic ?? []) {
      if (!seen.has(item.id)) {
        seen.add(item.id)
        all.push(item)
      }
    }
  }

  logLine(`Collected ${all.length} unique semantic memories`)
  return all
}

const main = async (): Promise<void> => {
  loadWorkspaceEnv()

  const {
    baseUrl,
    conversationId,
    outDir,
  } = getCliConfig()

  logLine(`Exporting memories for conversation: ${conversationId}`)
  logLine(`Output dir: ${outDir}`)
  logLine(`Base URL: ${baseUrl}`)

  const [episodic, semantic] = await Promise.all([
    exportEpisodicMemories(conversationId, baseUrl),
    exportSemanticMemories(conversationId, baseUrl),
  ])

  const episodicPath = resolve(outDir, 'episodic_memories.json')
  const semanticPath = resolve(outDir, 'semantic_memories.json')

  await writeFile(episodicPath, JSON.stringify(episodic, null, 2))
  await writeFile(semanticPath, JSON.stringify(semantic, null, 2))

  logLine(`Wrote ${episodic.length} episodic memories -> ${episodicPath}`)
  logLine(`Wrote ${semantic.length} semantic memories -> ${semanticPath}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((err) => {
  console.error(err)
  exit(1)
})
