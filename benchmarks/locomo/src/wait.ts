/* eslint-disable no-console */
import { sleep } from '@moeru/std'
import { benchmarkFlush, benchmarkJobStatus } from 'plastmem'

const POLL_INTERVAL_MS = 10_000
const ADMISSION_POLL_INTERVAL_MS = 1_000

export interface ConversationStatus {
  admissible_for_add: boolean
  done: boolean
  fence_active: boolean
  flushable: boolean
  messages_pending: number
  predict_calibrate_jobs_active: number
  segmentation_jobs_active: number
}

export const getStatus = async (
  baseUrl: string,
  conversationId: string,
): Promise<ConversationStatus> => {
  const res = await benchmarkJobStatus({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })
  return res.data as ConversationStatus
}

const renderStatus = (id: string, status: ConversationStatus): string => {
  const shortId = id.slice(0, 8)
  const phase = status.done ? 'done' : 'wait'
  return `${shortId}:p=${status.messages_pending},f=${status.fence_active ? 1 : 0},s=${status.segmentation_jobs_active},pc=${status.predict_calibrate_jobs_active},a=${status.admissible_for_add ? 1 : 0},fl=${status.flushable ? 1 : 0} ${phase}`
}

export const waitUntilConversationAdmissible = async (
  baseUrl: string,
  conversationId: string,
): Promise<void> => {
  while (true) {
    const status = await getStatus(baseUrl, conversationId)
    if (status.admissible_for_add)
      return

    await sleep(ADMISSION_POLL_INTERVAL_MS)
  }
}

export const flushConversationTailWhenReady = async (
  baseUrl: string,
  conversationId: string,
): Promise<boolean> => {
  while (true) {
    const status = await getStatus(baseUrl, conversationId)

    if (status.flushable) {
      await benchmarkFlush({
        baseUrl,
        body: { conversation_id: conversationId },
        throwOnError: true,
      })
      return true
    }

    if (status.messages_pending === 0 && !status.fence_active && status.segmentation_jobs_active === 0)
      return false

    await sleep(ADMISSION_POLL_INTERVAL_MS)
  }
}

export const waitForAll = async (
  conversationIds: string[],
  baseUrl: string,
): Promise<void> => {
  const uniqueIds = [...new Set(conversationIds.filter(id => id.length > 0))]
  if (uniqueIds.length === 0)
    return

  const pendingIds = new Set(uniqueIds)
  const flushedIds = new Set<string>()
  while (pendingIds.size > 0) {
    const ids = [...pendingIds]
    const statuses = await Promise.all(ids.map(async (id) => {
      const status = await getStatus(baseUrl, id)
      return { id, status }
    }))

    const line = statuses.map(({ id, status }) => renderStatus(id, status)).join(' | ')
    console.log(`  [wait] ${line}`)

    for (const { id, status } of statuses) {
      if (status.flushable && !flushedIds.has(id)) {
        const res = await benchmarkFlush({
          baseUrl,
          body: { conversation_id: id },
          throwOnError: true,
        })
        if (res.data?.enqueued === true)
          flushedIds.add(id)
      }
    }

    for (const { id, status } of statuses) {
      if (status.done)
        pendingIds.delete(id)
    }

    if (pendingIds.size === 0)
      break

    await sleep(POLL_INTERVAL_MS)
  }
}
