import type {
  BenchmarkRunConfig,
  RunManifest,
  SampleResultFile,
  SampleState,
} from './checkpoint'
import type { LoCoMoSample } from './types'

import z from 'zod'

const qACategorySchema = z.union([
  z.literal(1),
  z.literal(2),
  z.literal(3),
  z.literal(4),
  z.literal(5),
])

const dialogTurnSchema = z.object({
  blip_caption: z.string().optional(),
  compressed_text: z.string().optional(),
  dia_id: z.string(),
  img_file: z.string().optional(),
  search_query: z.string().optional(),
  speaker: z.string(),
  text: z.string(),
})

const qAPairSchema = z.object({
  adversarial_answer: z.string().nullable().optional(),
  answer: z.union([z.number(), z.string()]).optional(),
  category: qACategorySchema,
  evidence: z.array(z.string()),
  question: z.string(),
}).transform((value, context) => {
  if (value.answer != null)
    return value

  if (value.category === 5 && value.adversarial_answer != null)
    return { ...value, answer: value.adversarial_answer }

  context.addIssue({
    code: 'custom',
    message: 'QA pair is missing answer',
    path: ['answer'],
  })
  return z.NEVER
})

const qAResultSchema = z.object({
  category: qACategorySchema,
  context_retrieved: z.string(),
  evidence: z.array(z.string()),
  gold_answer: z.union([z.number(), z.string()]),
  llm_judge_score: z.number(),
  nemori_f1_score: z.number(),
  prediction: z.string(),
  question: z.string(),
  sample_id: z.string(),
  score: z.number(),
})

const benchmarkScoreSummarySchema = z.object({
  by_category: z.record(z.string(), z.number()),
  by_category_count: z.record(z.string(), z.number()),
  by_category_llm: z.record(z.string(), z.number()),
  by_category_nemori_f1: z.record(z.string(), z.number()),
  overall: z.number(),
  overall_llm: z.number(),
  overall_nemori_f1: z.number(),
  total: z.number(),
})

const benchmarkStatsSchema = z.object({
  by_sample: z.record(z.string(), benchmarkScoreSummarySchema),
  overall: benchmarkScoreSummarySchema,
})

const benchmarkVariantOutputSchema = z.object({
  results: z.array(qAResultSchema),
  stats: benchmarkStatsSchema,
})

const benchmarkRunConfigSchema = z.object({
  baseUrl: z.string(),
  compareFullContext: z.boolean(),
  dataFile: z.string(),
  model: z.string(),
  outDir: z.string(),
  sampleConcurrency: z.number().int().positive(),
  sampleIds: z.array(z.string()),
  seed: z.number().int().optional(),
  useLlmJudge: z.boolean(),
  waitForBackground: z.boolean(),
})

const sampleVariantStateSchema = z.object({
  eval_done: z.boolean(),
})

const sampleStateSchema = z.object({
  conversation_id: z.string().nullable(),
  error: z.string().nullable(),
  ingest_done: z.boolean(),
  sample_id: z.string(),
  status: z.union([
    z.literal('complete'),
    z.literal('failed'),
    z.literal('pending'),
    z.literal('running'),
  ]),
  updated_at: z.string(),
  variants: z.object({
    full_context: sampleVariantStateSchema.optional(),
    plastmem: sampleVariantStateSchema.optional(),
  }),
})

const sampleResultFileSchema = z.object({
  sample_id: z.string(),
  variants: z.object({
    full_context: benchmarkVariantOutputSchema.optional(),
    plastmem: benchmarkVariantOutputSchema.optional(),
  }),
})

const runManifestSchema = z.object({
  completed_at: z.string().nullable(),
  config: benchmarkRunConfigSchema,
  fingerprint: z.string(),
  sample_ids: z.array(z.string()),
  started_at: z.string(),
  updated_at: z.string(),
  version: z.literal(2),
})

const loCoMoSampleSchema = z.object({
  conversation: z.record(z.string(), z.union([z.string(), z.array(dialogTurnSchema)])),
  qa: z.array(qAPairSchema),
  sample_id: z.string(),
})

export const parseLoCoMoSamples = (value: unknown): LoCoMoSample[] =>
  z.array(loCoMoSampleSchema).parse(value) as LoCoMoSample[]

export const parseBenchmarkRunConfig = (value: unknown): BenchmarkRunConfig =>
  benchmarkRunConfigSchema.parse(value) as BenchmarkRunConfig

export const parseRunManifest = (value: unknown): RunManifest =>
  runManifestSchema.parse(value) as RunManifest

export const parseSampleState = (value: unknown): SampleState =>
  sampleStateSchema.parse(value) as SampleState

export const parseSampleResult = (value: unknown): SampleResultFile =>
  sampleResultFileSchema.parse(value) as SampleResultFile
