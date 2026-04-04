import type { LongMemEvalDataset } from '../types'

import { existsSync } from 'node:fs'
import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { env } from 'node:process'

import z from 'zod'

import { downloadFileToCacheDir, getHFHubCachePath, getRepoFolderName } from '@huggingface/hub'

import * as p from '@clack/prompts'

export const DATASET_REPO_ID = 'xiaowu0162/longmemeval-cleaned'
export const DATASET_FILE_ID = 'longmemeval_s_cleaned.json'

const longMemEvalTurnSchema = z.object({
  content: z.string(),
  has_answer: z.boolean().optional(),
  role: z.enum(['assistant', 'user']),
})

const answerSchema = z.union([z.number(), z.string()])

const longMemEvalSampleSchema = z.object({
  answer: answerSchema,
  answer_session_ids: z.array(z.string()),
  haystack_dates: z.array(z.string()),
  haystack_session_ids: z.array(z.string()),
  haystack_sessions: z.array(z.array(longMemEvalTurnSchema)),
  improved_answer: answerSchema.optional(),
  improved_question: z.string().optional(),
  improvement_note: z.string().optional(),
  question: z.string(),
  question_date: z.string(),
  question_id: z.string(),
  question_type: z.enum([
    'knowledge-update',
    'multi-session',
    'single-session-assistant',
    'single-session-preference',
    'single-session-user',
    'temporal-reasoning',
  ]),
  requires_retry: z.boolean().optional(),
})

const longMemEvalDatasetSchema = z.array(longMemEvalSampleSchema).min(1, 'LongMemEval dataset is empty.')

export const checkDataset = async (): Promise<string | undefined> => {
  const cacheDir = getHFHubCachePath()
  const repoFolderName = getRepoFolderName({ name: DATASET_REPO_ID, type: 'dataset' })

  const repoDir = join(cacheDir, repoFolderName)
  if (!existsSync(repoDir))
    return
  p.log.info(`repo: ${DATASET_REPO_ID}`)

  const repoSnapshotsDir = join(repoDir, 'snapshots')
  if (!existsSync(repoSnapshotsDir))
    return

  const revisions = await readdir(repoSnapshotsDir)
  if (revisions.length === 0)
    return
  p.log.info(`revision: ${revisions[0]}`)

  const filePath = join(repoDir, 'snapshots', revisions[0], DATASET_FILE_ID)
  if (!existsSync(filePath))
    return
  p.log.info(`file: ${DATASET_FILE_ID}`)

  return filePath
}

export const downloadDataset = async () => {
  let hfToken: string | undefined

  if (env.HF_TOKEN == null) {
    const cacheDir = getHFHubCachePath()
    const hfTokenPath = join(cacheDir, '..', 'token')
    try {
      if (existsSync(hfTokenPath)) {
        hfToken = await readFile(hfTokenPath, 'utf-8')
      }
    }
    catch {}
  }

  return downloadFileToCacheDir({
    accessToken: env.HF_TOKEN ?? hfToken,
    path: DATASET_FILE_ID,
    repo: {
      name: DATASET_REPO_ID,
      type: 'dataset',
    },
  })
}

export const loadDataset = async (path: string): Promise<LongMemEvalDataset> => {
  const raw = await readFile(path, 'utf-8')
  const parsed: unknown = JSON.parse(raw)

  return longMemEvalDatasetSchema.parse(parsed) as LongMemEvalDataset
}
