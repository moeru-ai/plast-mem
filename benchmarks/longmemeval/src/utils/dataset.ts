import type { LongMemEvalDataset } from '../types'

import { existsSync } from 'node:fs'
import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { env } from 'node:process'

import { downloadFileToCacheDir, getHFHubCachePath, getRepoFolderName } from '@huggingface/hub'

import * as p from '@clack/prompts'

const REPO_ID = 'xiaowu0162/longmemeval-cleaned'
const FILE_ID = 'longmemeval_m_cleaned.json'

export const checkDataset = async (): Promise<string | undefined> => {
  const cacheDir = getHFHubCachePath()
  const repoFolderName = getRepoFolderName({ name: REPO_ID, type: 'dataset' })

  const repoDir = join(cacheDir, repoFolderName)
  if (!existsSync(repoDir))
    return
  p.log.info(`repo: ${REPO_ID}`)

  const repoSnapshotsDir = join(repoDir, 'snapshots')
  if (!existsSync(repoSnapshotsDir))
    return

  const revisions = await readdir(repoSnapshotsDir)
  if (revisions.length === 0)
    return
  p.log.info(`revision: ${revisions[0]}`)

  const filePath = join(repoDir, 'snapshots', revisions[0], FILE_ID)
  if (!existsSync(filePath))
    return
  p.log.info(`file: ${FILE_ID}`)

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
    accessToken: env.HF_TOKEN ?? hfToken, // TODO: loadEnvFile
    path: FILE_ID,
    repo: {
      name: REPO_ID,
      type: 'dataset',
    },
  })
}

export const loadDataset = async (path: string): Promise<LongMemEvalDataset> => {
  const raw = await readFile(path, 'utf-8')
  const parsed: unknown = JSON.parse(raw)

  if (!Array.isArray(parsed)) {
    throw new TypeError(`Expected ${FILE_ID} to be a JSON array.`)
  }

  return parsed as LongMemEvalDataset
}
