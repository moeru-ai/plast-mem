import type { RunCheckpoint } from './checkpoint'
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

const benchmarkRunConfigSchema = z.object({
  baseUrl: z.string(),
  compareFullContext: z.boolean(),
  dataFile: z.string(),
  model: z.string(),
  outFile: z.string(),
  sampleIds: z.array(z.string()),
  seed: z.number().int().optional(),
  useLlmJudge: z.boolean(),
  waitForBackground: z.boolean(),
})

const variantCheckpointSchema = z.object({
  eval_done: z.boolean(),
  results: z.array(qAResultSchema),
})

const sampleCheckpointSchema = z.object({
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
  variants: z.object({
    full_context: variantCheckpointSchema.optional(),
    plastmem: variantCheckpointSchema.optional(),
  }),
})

const runCheckpointSchema = z.object({
  completed_at: z.string().nullable(),
  config: benchmarkRunConfigSchema,
  fingerprint: z.string(),
  samples: z.record(z.string(), sampleCheckpointSchema),
  started_at: z.string(),
  updated_at: z.string(),
  version: z.literal(1),
})

const loCoMoSampleSchema = z.object({
  conversation: z.record(z.string(), z.union([z.string(), z.array(dialogTurnSchema)])),
  qa: z.array(qAPairSchema),
  sample_id: z.string(),
})

export const parseLoCoMoSamples = (value: unknown): LoCoMoSample[] =>
  z.array(loCoMoSampleSchema).parse(value) as LoCoMoSample[]

export const parseRunCheckpoint = (value: unknown): RunCheckpoint =>
  runCheckpointSchema.parse(value) as RunCheckpoint
