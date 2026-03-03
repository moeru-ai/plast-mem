import type { BenchmarkOutput, LoCoMoSample, QAResult } from './types'

import process from 'node:process'

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { scoreAnswer } from './evaluation'
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
  dataFile: string
  outFile: string
  sampleIds: null | string[]
  skipIngest: boolean
  skipWait: boolean
}

const parseArgs = (): Args => {
  const argv = process.argv.slice(2)
  const get = (flag: string, fallback: string): string => {
    const i = argv.indexOf(flag)
    if (i === -1 || i + 1 >= argv.length)
      return fallback
    return argv[i + 1]
  }
  const sampleIdStr = get('--sample-ids', '')
  return {
    dataFile: get('--data-file', resolve(__dirname, '../data/locomo10.json')),
    outFile: get('--out-file', resolve(__dirname, `../results/${new Date().toISOString().replace(/[:.]/g, '-')}.json`)),
    sampleIds: sampleIdStr.length > 0 ? sampleIdStr.split(',').map(s => s.trim()) : null,
    skipIngest: argv.includes('--skip-ingest'),
    skipWait: argv.includes('--skip-wait'),
  }
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

  console.log('LoCoMo Benchmark for plast-mem')
  console.log(`  data:    ${args.dataFile}`)
  console.log(`  out:     ${args.outFile}`)
  console.log(`  model:   ${model}`)
  console.log(`  baseUrl: ${baseUrl}`)
  console.log('')

  const raw = readFileSync(args.dataFile, 'utf-8')
  const allSamples = JSON.parse(raw) as LoCoMoSample[]
  const sampleIds = args.sampleIds
  const samples = sampleIds != null
    ? allSamples.filter(s => sampleIds.includes(s.sample_id))
    : allSamples

  console.log(`Loaded ${samples.length} sample(s).`)

  const idsFile = resolve(__dirname, '../data/conversation_ids.json')

  // Step 1: Ingest
  let conversationIds: Record<string, string>
  if (!args.skipIngest) {
    console.log('\n── Step 1: Ingesting conversations ──')
    conversationIds = await ingestAll(samples, baseUrl)
    saveConversationIds(idsFile, conversationIds)
    console.log('Ingestion complete.')
  }
  else {
    console.log('Skipping ingestion (--skip-ingest).')
    conversationIds = loadConversationIds(idsFile)
  }

  // Step 2: Wait
  const ids = samples.map(s => conversationIds[s.sample_id]).filter(Boolean)
  if (!args.skipWait) {
    console.log('\n── Step 2: Waiting for background processing ──')
    await waitForAll(baseUrl, ids)
    console.log('All conversations processed.')
  }
  else {
    console.log('Skipping wait (--skip-wait).')
  }

  // Step 3: Evaluate
  console.log('\n── Step 3: Evaluating QA ──')
  const results: QAResult[] = []

  for (const sample of samples) {
    const conversationId = conversationIds[sample.sample_id]
    if (!conversationId) {
      console.warn(`  No conversation_id for sample ${sample.sample_id}, skipping.`)
      continue
    }

    const qaCount = sample.qa.length
    console.log(`  Sample ${sample.sample_id}: ${qaCount} questions`)

    // Prefetch all contexts in parallel (HTTP to plast-mem, not Ollama)
    process.stdout.write(`  Prefetching ${qaCount} contexts...`)
    const contexts = await Promise.all(sample.qa.map(async qa => getContext(conversationId, qa.question, baseUrl)))
    process.stdout.write(' done\n')

    for (let i = 0; i < sample.qa.length; i++) {
      const qa = sample.qa[i]
      const context = contexts[i] ?? ''
      process.stdout.write(`    [${i + 1}/${qaCount}] generating...`)

      const prediction = await generateAnswer(context, qa.question, qa.category, model)
      const score = scoreAnswer(prediction, qa.answer, qa.category)
      // const llmScore = await llmJudge(prediction, qa.answer, qa.question, model)
      const llmScore = 0

      process.stdout.write(` f1=${score.toFixed(2)}\n`)

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
    }
  }

  // Step 4: Stats
  const stats = computeStats(results)
  printStats(stats)

  const output: BenchmarkOutput = {
    meta: { base_url: baseUrl, data_file: args.dataFile, model, timestamp: new Date().toISOString() },
    results,
    stats,
  }

  mkdirSync(dirname(args.outFile), { recursive: true })
  writeFileSync(args.outFile, JSON.stringify(output, null, 2))
  console.log(`Results written to: ${args.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((err) => {
  console.error(err)
  process.exit(1)
})
