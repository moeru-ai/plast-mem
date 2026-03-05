import { stdout } from 'node:process'

import { sleep } from '@moeru/std'
import { benchmarkJobStatus } from 'plastmem'

const INITIAL_WAIT_MS = 2 * 60_000
const POLL_INTERVAL_MS = 10_000

interface ConversationStatus {
  apalis_active: number
  done: boolean
  fence_active: boolean
  messages_pending: number
}

const getStatus = async (
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
  return `${shortId}:p=${status.messages_pending},f=${status.fence_active ? 1 : 0},a=${status.apalis_active} ${phase}`
}

export const waitForAll = async (
  conversationIds: string[],
  baseUrl: string,
): Promise<void> => {
  const uniqueIds = [...new Set(conversationIds.filter(id => id.length > 0))]
  if (uniqueIds.length === 0)
    return

  stdout.write('  Waiting 2 minutes before polling background jobs...\n')
  await sleep(INITIAL_WAIT_MS)

  const pendingIds = new Set(uniqueIds)
  while (pendingIds.size > 0) {
    const ids = [...pendingIds]
    const statuses = await Promise.all(ids.map(async (id) => {
      const status = await getStatus(baseUrl, id)
      return { id, status }
    }))

    const line = statuses.map(({ id, status }) => renderStatus(id, status)).join(' | ')
    stdout.write(`  [wait] ${line}\n`)

    for (const { id, status } of statuses) {
      if (status.done)
        pendingIds.delete(id)
    }

    if (pendingIds.size === 0)
      break

    await sleep(POLL_INTERVAL_MS)
  }
}
