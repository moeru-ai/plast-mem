import type { BenchmarkOutput, LoCoMoSample, QAResult } from './types'

import process from 'node:process'

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { llmJudge, scoreAnswer } from './evaluation'
import { ingestAll, loadConversationIds, saveConversationIds } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import { computeStats, printStats } from './stats'
import { waitForAll } from './wait'

const __dirname = dirname(fileURLToPath(import.meta.url))

// ──────────────────────────────────────────────────
// CLI argument parsing
// ──────────────────────────────────────────────────

interface Args {
  concurrency: number
  dataFile: string
  outFile: string
  sampleIds: null | string[]
  skipIngest: boolean
  useLlmJudge: boolean
}

const parseArgs = (): Args => {
  const argv = process.argv.slice(2)
  const get = (flag: string, fallback: string): string => {
    const i = argv.indexOf(flag)
    if (i === -1 || i + 1 >= argv.length)
      return fallback
    return argv[i + 1]
  }
  const getInt = (flag: string, fallback: number): number => {
    const raw = get(flag, String(fallback))
    const value = Number.parseInt(raw, 10)
    return Number.isFinite(value) && value > 0 ? value : fallback
  }
  const sampleIdStr = get('--sample-ids', '')
  return {
    concurrency: getInt('--concurrency', 4),
    dataFile: get('--data-file', resolve(__dirname, '../data/locomo10.json')),
    outFile: get('--out-file', resolve(__dirname, `../results/${new Date().toISOString().replace(/[:.]/g, '-')}.json`)),
    sampleIds: sampleIdStr.length > 0 ? sampleIdStr.split(',').map(s => s.trim()) : null,
    skipIngest: argv.includes('--skip-ingest'),
    useLlmJudge: argv.includes('--use-llm-judge'),
  }
}

const runWithConcurrency = async (
  tasks: Array<() => Promise<void>>,
  concurrency: number,
): Promise<void> => {
  if (tasks.length === 0)
    return

  const limit = Math.max(1, Math.floor(concurrency))
  let nextIndex = 0

  const worker = async (): Promise<void> => {
    while (true) {
      const i = nextIndex
      nextIndex += 1
      if (i >= tasks.length)
        return
      await tasks[i]()
    }
  }

  await Promise.all(Array.from(
    { length: Math.min(limit, tasks.length) },
    async () => {
      await worker()
    },
  ))
}

const writeLine = (line: string): void => {
  process.stdout.write(`${line}\n`)
}

// ──────────────────────────────────────────────────
// Main
// ──────────────────────────────────────────────────

const main = async () => {
  // Load root .env before reading env vars
  try {
    process.loadEnvFile(resolve(__dirname, '../../../.env'))
  }
  catch { }

  const args = parseArgs()
  const baseUrl = (process.env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(/\/$/, '')
  const model = process.env.OPENAI_CHAT_MODEL ?? 'gpt-4o-mini'

  if (process.env.OPENAI_API_KEY == null || process.env.OPENAI_API_KEY.length === 0) {
    console.error('Error: OPENAI_API_KEY not set.')
    process.exit(1)
  }

  writeLine('LoCoMo Benchmark for plast-mem')
  writeLine(`  data:    ${args.dataFile}`)
  writeLine(`  out:     ${args.outFile}`)
  writeLine(`  model:   ${model}`)
  writeLine(`  baseUrl: ${baseUrl}`)
  writeLine(`  concurrency: ${args.concurrency}`)
  writeLine(`  llmJudge: ${args.useLlmJudge ? 'on' : 'off'}`)
  writeLine('')

  const raw = await readFile(args.dataFile, 'utf-8')
  const allSamples = JSON.parse(raw) as LoCoMoSample[]
  const sampleIds = args.sampleIds
  const samples = sampleIds != null
    ? allSamples.filter(s => sampleIds.includes(s.sample_id))
    : allSamples

  writeLine(`Loaded ${samples.length} sample(s).`)

  const idsFile = resolve(__dirname, '../data/conversation_ids.json')

  // Step 1: Ingest
  let conversationIds: Record<string, string>
  if (!args.skipIngest) {
    writeLine('\n── Step 1: Ingesting conversations ──')
    conversationIds = await ingestAll(samples, baseUrl)
    await saveConversationIds(idsFile, conversationIds)
    writeLine('Ingestion complete.')
  }
  else {
    writeLine('Skipping ingestion (--skip-ingest).')
    conversationIds = await loadConversationIds(idsFile)
  }

  // Step 2: Wait
  writeLine('\n── Step 2: Waiting for background processing ──')
  const activeConversationIds = samples
    .map(sample => conversationIds[sample.sample_id])
    .filter((id): id is string => id != null && id.length > 0)
  await waitForAll(activeConversationIds, baseUrl)
  writeLine('Background processing complete.')

  // Step 3: Evaluate
  writeLine('\n── Step 3: Evaluating QA ──')
  const results: QAResult[] = []

  for (const sample of samples) {
    const conversationId = conversationIds[sample.sample_id]
    if (!conversationId) {
      console.warn(`  No conversation_id for sample ${sample.sample_id}, skipping.`)
      continue
    }

    const qaCount = sample.qa.length
    writeLine(`  Sample ${sample.sample_id}: ${qaCount} questions`)

    // Prefetch contexts with bounded concurrency to avoid overloading embedding backend.
    process.stdout.write(`  Prefetching ${qaCount} contexts...`)
    const contexts: string[] = Array.from({ length: qaCount }, () => '')
    const contextTasks = sample.qa.map((qa, index) => async () => {
      contexts[index] = await getContext(conversationId, qa.question, baseUrl)
    })
    await runWithConcurrency(contextTasks, args.concurrency)
    process.stdout.write(' done\n')

    const buffered: Array<null | {
      context: string
      llmScore: number
      prediction: string
      qa: (typeof sample.qa)[number]
      score: number
    }> = Array.from({ length: qaCount }, () => null)
    let nextToPrint = 0

    const flush = () => {
      while (nextToPrint < qaCount && buffered[nextToPrint] != null) {
        const { context, llmScore, prediction, qa, score } = buffered[nextToPrint]!
        process.stdout.write(`    [${nextToPrint + 1}/${qaCount}] generating... f1=${score.toFixed(2)}\n`)

        results.push({
          category: qa.category,
          context_retrieved: context,
          evidence: qa.evidence,
          gold_answer: qa.answer as string,
          llm_judge_score: llmScore,
          prediction,
          question: qa.question,
          sample_id: sample.sample_id,
          score,
        })

        buffered[nextToPrint] = null
        nextToPrint++
      }
    }

    const tasks = sample.qa.map((qa, index) => async () => {
      const context = contexts[index] ?? ''
      const prediction = await generateAnswer(context, qa.question, qa.category, model)
      const score = scoreAnswer(prediction, qa.answer, qa.category)
      const llmScore = args.useLlmJudge && qa.category !== 5
        ? await llmJudge(prediction, qa.answer, qa.question, model)
        : 0
      buffered[index] = { context, llmScore, prediction, qa, score }
      flush()
    })

    await runWithConcurrency(tasks, args.concurrency)
    flush()
  }

  // Step 4: Stats
  const stats = computeStats(results)
  printStats(stats)

  const output: BenchmarkOutput = {
    meta: { base_url: baseUrl, data_file: args.dataFile, model, timestamp: new Date().toISOString() },
    results,
    stats,
  }

  await mkdir(dirname(args.outFile), { recursive: true })
  await writeFile(args.outFile, JSON.stringify(output, null, 2))
  writeLine(`Results written to: ${args.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((err) => {
  console.error(err)
  process.exit(1)
})
