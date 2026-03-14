import type { BenchmarkOutput, LoCoMoSample, QAResult } from './types'

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { env, exit, loadEnvFile } from 'node:process'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

import { Spinner } from 'picospinner'

import { llmJudge, scoreAnswer, scoreAnswerNemoriF1 } from './evaluation'
import { ingestAll, loadConversationIds, saveConversationIds } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import { computeStats, printStats } from './stats'
import { waitForAll } from './wait'

const __dirname = dirname(fileURLToPath(import.meta.url))

interface Args {
  concurrency: number
  dataFile: string
  outFile: string
  sampleIds: null | string[]
  skipIngest: boolean
  skipWait: boolean
  useLlmJudge: boolean
}

type ArtifactOutput = Omit<BenchmarkOutput, 'results'> & {
  results: ArtifactResult[]
}

type ArtifactResult = Omit<QAResult, 'llm_judge_score' | 'nemori_f1_score' | 'score'> & {
  llm_judge_score: null | number
  nemori_f1_score: null | number
  score: null | number
}

const parseCliArgs = (): Args => {
  const { values } = parseArgs({
    options: {
      'concurrency': {
        default: '2',
        short: 'c',
        type: 'string',
      },
      'data-file': {
        short: 'd',
        type: 'string',
      },
      'out-file': {
        short: 'o',
        type: 'string',
      },
      'sample-ids': {
        short: 's',
        type: 'string',
      },
      'skip-ingest': {
        default: false,
        type: 'boolean',
      },
      'skip-wait': {
        default: false,
        type: 'boolean',
      },
      'use-llm-judge': {
        default: false,
        type: 'boolean',
      },
    },
  })

  const concurrency = Number.parseInt(values.concurrency, 10)
  const sampleIdStr = values['sample-ids'] ?? ''

  return {
    concurrency: Number.isFinite(concurrency) && concurrency > 0 ? concurrency : 2,
    dataFile: values['data-file'] ?? resolve(__dirname, '../data/locomo10.json'),
    outFile: values['out-file'] ?? resolve(__dirname, `../results/${new Date().toISOString().replace(/[:.]/g, '-')}.json`),
    sampleIds: sampleIdStr.length > 0 ? sampleIdStr.split(',').map(s => s.trim()) : null,
    skipIngest: values['skip-ingest'],
    skipWait: values['skip-wait'],
    useLlmJudge: values['use-llm-judge'],
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

const buildMeta = (baseUrl: string, dataFile: string, model: string): BenchmarkOutput['meta'] => ({
  base_url: baseUrl,
  data_file: dataFile,
  model,
  timestamp: new Date().toISOString(),
})

const isEvaluatedResult = (result: ArtifactResult): result is QAResult =>
  result.llm_judge_score != null
  && result.nemori_f1_score != null
  && result.score != null

const getEvaluatedResults = (results: ArtifactResult[]): QAResult[] =>
  results.filter(isEvaluatedResult)

const writeArtifact = async (
  outFile: string,
  baseUrl: string,
  dataFile: string,
  model: string,
  results: ArtifactResult[],
): Promise<void> => {
  const output: ArtifactOutput = {
    meta: buildMeta(baseUrl, dataFile, model),
    results,
    stats: computeStats(getEvaluatedResults(results)),
  }

  await mkdir(dirname(outFile), { recursive: true })
  await writeFile(outFile, JSON.stringify(output, null, 2))
}

const runQaStage = async (
  samples: LoCoMoSample[],
  conversationIds: Record<string, string>,
  baseUrl: string,
  model: string,
  args: Args,
): Promise<ArtifactResult[]> => {
  console.log('\n── Step 2: QA ──')

  const results: ArtifactResult[] = []

  for (const sample of samples) {
    const conversationId = conversationIds[sample.sample_id]
    if (!conversationId) {
      console.warn(`  No conversation_id for sample ${sample.sample_id}, skipping.`)
      continue
    }

    const qaPairs = sample.qa.filter(qa => qa.category !== 5)
    const qaCount = qaPairs.length
    console.log(`  Sample ${sample.sample_id}: ${qaCount} questions`)
    if (qaCount === 0) {
      await writeArtifact(args.outFile, baseUrl, args.dataFile, model, results)
      continue
    }

    const prefetchSpinner = new Spinner(`Prefetching ${qaCount} contexts`)
    prefetchSpinner.start()
    const contexts: string[] = Array.from({ length: qaCount }, () => '')
    const contextTasks = qaPairs.map((qa, index) => async () => {
      contexts[index] = await getContext(conversationId, qa.question, baseUrl)
    })
    await runWithConcurrency(contextTasks, args.concurrency)
    prefetchSpinner.succeed(`Prefetched ${qaCount} contexts`)

    const buffered: Array<ArtifactResult | null> = Array.from({ length: qaCount }, () => null)
    let nextToPrint = 0

    const flush = () => {
      while (nextToPrint < qaCount && buffered[nextToPrint] != null) {
        const result = buffered[nextToPrint]!
        console.log(`    [${nextToPrint + 1}/${qaCount}] answering...`)
        results.push(result)
        buffered[nextToPrint] = null
        nextToPrint++
      }
    }

    const tasks = qaPairs.map((qa, index) => async () => {
      const context = contexts[index] ?? ''
      const prediction = await generateAnswer(context, qa.question, qa.category, model)
      buffered[index] = {
        category: qa.category,
        context_retrieved: context,
        evidence: qa.evidence,
        gold_answer: qa.answer,
        llm_judge_score: null,
        nemori_f1_score: null,
        prediction,
        question: qa.question,
        sample_id: sample.sample_id,
        score: null,
      }
      flush()
    })

    await runWithConcurrency(tasks, args.concurrency)
    flush()

    await writeArtifact(args.outFile, baseUrl, args.dataFile, model, results)
    console.log(`  Artifact updated: ${args.outFile}`)
  }

  return results
}

const runEvalStage = async (
  results: ArtifactResult[],
  baseUrl: string,
  model: string,
  args: Args,
): Promise<QAResult[]> => {
  console.log('\n── Step 3: Eval ──')

  let startIndex = 0
  while (startIndex < results.length) {
    const sampleId = results[startIndex]?.sample_id
    if (sampleId == null)
      break

    let endIndex = startIndex
    while (endIndex < results.length && results[endIndex]?.sample_id === sampleId)
      endIndex++

    const sampleResults = results.slice(startIndex, endIndex)
    console.log(`  Sample ${sampleId}: ${sampleResults.length} questions`)

    const buffered: Array<null | { index: number, result: QAResult }> = Array.from(
      { length: sampleResults.length },
      () => null,
    )
    let nextToPrint = 0

    const flush = () => {
      while (nextToPrint < sampleResults.length && buffered[nextToPrint] != null) {
        const { index, result } = buffered[nextToPrint]!
        console.log(
          `    [${nextToPrint + 1}/${sampleResults.length}] scoring... `
          + `f1=${result.score.toFixed(2)} `
          + `nemoriF1=${result.nemori_f1_score.toFixed(2)} `
          + `llm=${result.llm_judge_score.toFixed(2)}`,
        )

        results[index] = result
        buffered[nextToPrint] = null
        nextToPrint++
      }
    }

    const tasks = sampleResults.map((sampleResult, sampleIndex) => async () => {
      const score = scoreAnswer(sampleResult.prediction, sampleResult.gold_answer, sampleResult.category)
      const nemoriF1Score = scoreAnswerNemoriF1(sampleResult.prediction, sampleResult.gold_answer)
      const llmScore = args.useLlmJudge
        ? await llmJudge(sampleResult.prediction, sampleResult.gold_answer, sampleResult.question, model)
        : 0

      buffered[sampleIndex] = {
        index: startIndex + sampleIndex,
        result: {
          ...sampleResult,
          llm_judge_score: llmScore,
          nemori_f1_score: nemoriF1Score,
          score,
        },
      }
      flush()
    })

    await runWithConcurrency(tasks, args.concurrency)
    flush()

    await writeArtifact(args.outFile, baseUrl, args.dataFile, model, results)
    console.log(`  Artifact updated: ${args.outFile}`)
    startIndex = endIndex
  }

  return getEvaluatedResults(results)
}

const main = async () => {
  try {
    loadEnvFile(resolve(__dirname, '../../../.env'))
  }
  catch { }

  const args = parseCliArgs()
  const baseUrl = (env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(/\/$/, '')
  const model = env.OPENAI_CHAT_MODEL ?? 'gpt-4o-mini'

  if (env.OPENAI_API_KEY == null || env.OPENAI_API_KEY.length === 0) {
    console.error('Error: OPENAI_API_KEY not set.')
    exit(1)
  }

  console.log('LoCoMo Benchmark for plast-mem')
  console.log(`  data:    ${args.dataFile}`)
  console.log(`  out:     ${args.outFile}`)
  console.log(`  model:   ${model}`)
  console.log(`  baseUrl: ${baseUrl}`)
  console.log(`  concurrency: ${args.concurrency}`)
  console.log(`  llmJudge: ${args.useLlmJudge ? 'on' : 'off'}`)
  console.log()

  const raw = await readFile(args.dataFile, 'utf-8')
  const allSamples = JSON.parse(raw) as LoCoMoSample[]
  const samples = args.sampleIds != null
    ? allSamples.filter(sample => args.sampleIds!.includes(sample.sample_id))
    : allSamples

  console.log(`Loaded ${samples.length} sample(s).`)

  const idsFile = resolve(__dirname, '../data/conversation_ids.json')

  console.log('\n── Step 1: Ingest ──')
  let conversationIds: Record<string, string>
  if (!args.skipIngest) {
    console.log('  Ingesting conversations...')
    conversationIds = await ingestAll(samples, baseUrl)
    await saveConversationIds(idsFile, conversationIds)
    console.log('  Ingestion complete.')
  }
  else {
    console.log('  Skipping ingestion (--skip-ingest).')
    conversationIds = await loadConversationIds(idsFile)
  }

  console.log('  Waiting for background processing...')
  if (args.skipWait) {
    console.log('  Skipping wait (--skip-wait).')
  }
  else {
    const activeConversationIds = samples
      .map(sample => conversationIds[sample.sample_id])
      .filter((id): id is string => id != null && id.length > 0)
    await waitForAll(activeConversationIds, baseUrl)
    console.log('  Background processing complete.')
  }

  const artifactResults = await runQaStage(samples, conversationIds, baseUrl, model, args)
  const evalResults = await runEvalStage(artifactResults, baseUrl, model, args)

  printStats(computeStats(evalResults))
  console.log(`Results written to: ${args.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((err) => {
  console.error(err)
  exit(1)
})
