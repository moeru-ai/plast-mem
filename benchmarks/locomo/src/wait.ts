import { spinner as createSpinner } from '@clack/prompts'
import { sleep } from '@moeru/std'
import { benchmarkJobStatus } from 'plastmem'

const POLL_INTERVAL_MS = 10_000

export interface ConversationStatus {
  done: boolean
  fence_active: boolean
  messages_pending: number
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
  const res = await benchmarkJobStatus({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })
  return res.data as ConversationStatus
}

const renderFlag = (value: boolean): string => value ? 'yes' : 'no'

const renderStatus = (
  index: number,
  total: number,
  status: ConversationStatus,
): string => {
  const prefix = total > 1 ? `conversation ${index + 1} ` : ''
  return `${prefix}pending=${status.messages_pending}, `
    + `fence=${renderFlag(status.fence_active)}, `
    + `segmentation=${status.segmentation_jobs_active}, `
    + `predict_calibrate=${status.predict_calibrate_jobs_active}`
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
  const spinner = createSpinner()
  spinner.start(uniqueIds.length === 1 ? 'Waiting for background jobs' : `Waiting for ${uniqueIds.length} conversations`)
  while (pendingIds.size > 0) {
    const statuses = await collectStatuses([...pendingIds], baseUrl)

    for (const { id, status } of statuses)
      observer?.onStatus?.(id, status)

    const line = statuses
      .map(({ status }, index) => renderStatus(index, statuses.length, status))
      .join(' | ')
    spinner.message(line)

    removeCompletedConversations(statuses, pendingIds)

    if (pendingIds.size === 0) {
      spinner.stop(uniqueIds.length === 1 ? 'Background jobs settled' : 'All background jobs settled')
      break
    }

    await sleep(POLL_INTERVAL_MS)
  }
}
