import process from 'node:process'

const POLL_INTERVAL_MS = 10_000 // 10s between polls
const POLL_TIMEOUT_MS = 10 * 60_000 // 10 min max polling
const INITIAL_WAIT_MS = 2 * 60_000 // 2 min initial wait

interface JobStatus {
  apalis_active: number
  done: boolean
  fence_active: boolean
  messages_pending: number
}

const getJobStatus = async (baseUrl: string, conversationId: string): Promise<JobStatus> => {
  const url = `${baseUrl}/api/v0/benchmark/job_status?conversation_id=${conversationId}`
  const res = await fetch(url)
  if (!res.ok)
    throw new Error(`job_status failed: ${res.status} ${await res.text()}`)
  return res.json() as Promise<JobStatus>
}

const triggerFlush = async (baseUrl: string, conversationId: string): Promise<boolean> => {
  const res = await fetch(`${baseUrl}/api/v0/benchmark/flush`, {
    body: JSON.stringify({ conversation_id: conversationId }),
    headers: { 'Content-Type': 'application/json' },
    method: 'POST',
  })
  if (!res.ok)
    throw new Error(`benchmark/flush failed: ${res.status} ${await res.text()}`)
  const body = (await res.json()) as { enqueued: boolean }
  return body.enqueued
}

/**
 * Poll all conversations in parallel until each is done or stuck.
 * Returns the set of conversation IDs that are stuck (need a flush).
 */
const pollAllUntilDoneOrStuck = async (
  baseUrl: string,
  conversationIds: string[],
  label: string,
): Promise<Set<string>> => {
  const stuck = new Set<string>()
  const pending = new Set(conversationIds)
  const deadline = Date.now() + POLL_TIMEOUT_MS

  while (pending.size > 0 && Date.now() < deadline) {
    const statuses = await Promise.all(
      [...pending].map(async id => ({ id, status: await getJobStatus(baseUrl, id) })),
    )

    const summary = statuses.map(({ id, status }) =>
      `${id.slice(0, 8)}: p=${status.messages_pending} f=${status.fence_active} a=${status.apalis_active}`,
    ).join(' | ')
    process.stdout.write(`\r  [${label}] ${summary}   `)

    for (const { id, status } of statuses) {
      if (status.done) {
        pending.delete(id)
      }
      else if (status.messages_pending > 0 && !status.fence_active && status.apalis_active === 0) {
        pending.delete(id)
        stuck.add(id)
      }
    }

    if (pending.size > 0) {
      await new Promise<void>((resolve) => {
        const timer = setTimeout(resolve, POLL_INTERVAL_MS)
        void timer
      })
    }
  }

  process.stdout.write('\n')
  if (pending.size > 0)
    console.warn(`  Warning: poll timeout for ${[...pending].join(', ')}`)

  return stuck
}

/**
 * Wait for all conversations to finish processing:
 * 1. Single 2-minute wait for auto-triggered jobs to fire.
 * 2. Poll all conversations in parallel until done or stuck.
 * 3. Flush all stuck conversations in parallel, then poll again.
 */
export const waitForAll = async (
  baseUrl: string,
  conversationIds: string[],
): Promise<void> => {
  // Phase 1: shared initial wait
  process.stdout.write('Phase 1: waiting 2 min for auto-triggered jobs...\n')
  await new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, INITIAL_WAIT_MS)
    void timer
  })

  // Phase 2+: poll → flush stuck → repeat until all done
  let toCheck = conversationIds
  let round = 2
  while (toCheck.length > 0) {
    process.stdout.write(`Phase ${round}: polling ${toCheck.length} conversation(s)...\n`)
    const stuck = await pollAllUntilDoneOrStuck(baseUrl, toCheck, `round-${round}`)
    if (stuck.size === 0)
      break
    process.stdout.write(`Phase ${round} flush: flushing ${stuck.size} stuck conversation(s)...\n`)
    await Promise.all([...stuck].map(async id => triggerFlush(baseUrl, id)))
    toCheck = [...stuck]
    round++
  }
}
