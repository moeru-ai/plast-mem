import { sleep } from '@moeru/std'
import { segmentationState } from 'plastmem'

const POLL_INTERVAL_MS = 10_000

export interface ConversationStatus {
  admissible_for_add: boolean
  done: boolean
  eof_seen: boolean
  fence_active: boolean
  last_seen_seq?: null | number
  messages_pending: number
  next_unsegmented_seq: number
  predict_calibrate_jobs_active: number
  segmentation_jobs_active: number
}

export interface WaitObserver {
  onStatus?: (conversationId: string, status: ConversationStatus) => void
}

interface StatusEntry {
  id: string
  status: ConversationStatus
}

export const getStatus = async (
  baseUrl: string,
  conversationId: string,
): Promise<ConversationStatus> => {
  const res = await segmentationState({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })
  return res.data as ConversationStatus
}

const collectStatuses = async (
  ids: string[],
  baseUrl: string,
): Promise<StatusEntry[]> =>
  Promise.all(ids.map(async (id) => {
    const status = await getStatus(baseUrl, id)
    return { id, status }
  }))

const removeCompletedConversations = (
  statuses: StatusEntry[],
  pendingIds: Set<string>,
): void => {
  for (const { id, status } of statuses) {
    if (status.done)
      pendingIds.delete(id)
  }
}

export const waitForAll = async (
  conversationIds: string[],
  baseUrl: string,
  observer?: WaitObserver,
): Promise<void> => {
  const uniqueIds = [...new Set(conversationIds.filter(id => id.length > 0))]
  if (uniqueIds.length === 0)
    return

  const pendingIds = new Set(uniqueIds)

  while (pendingIds.size > 0) {
    const statuses = await collectStatuses([...pendingIds], baseUrl)

    for (const { id, status } of statuses)
      observer?.onStatus?.(id, status)

    removeCompletedConversations(statuses, pendingIds)

    if (pendingIds.size === 0)
      break

    await sleep(POLL_INTERVAL_MS)
  }
}
