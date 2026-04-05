import { spinner as createSpinner } from '@clack/prompts'
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
    + `predict_calibrate=${status.predict_calibrate_jobs_active}, `
    + `admissible=${renderFlag(status.admissible_for_add)}, `
    + `eof=${renderFlag(status.eof_seen)}`
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
): Promise<void> => {
  const uniqueIds = [...new Set(conversationIds.filter(id => id.length > 0))]
  if (uniqueIds.length === 0)
    return

  const pendingIds = new Set(uniqueIds)
  const spinner = createSpinner()
  spinner.start(uniqueIds.length === 1 ? 'Waiting for background jobs' : `Waiting for ${uniqueIds.length} conversations`)
  while (pendingIds.size > 0) {
    const statuses = await collectStatuses([...pendingIds], baseUrl)

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
