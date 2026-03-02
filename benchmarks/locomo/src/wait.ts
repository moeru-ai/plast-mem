import process from 'node:process'

const POLL_INTERVAL_MS = 10_000 // 10s between polls
const POLL_TIMEOUT_MS = 10 * 60_000 // 10 min max polling

interface JobStatus {
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

const pollUntilDone = async (
  baseUrl: string,
  conversationId: string,
  label: string,
): Promise<void> => {
  const deadline = Date.now() + POLL_TIMEOUT_MS
  while (Date.now() < deadline) {
    const status = await getJobStatus(baseUrl, conversationId)
    process.stdout.write(
      `\r  ${label}: pending=${status.messages_pending} fence=${status.fence_active}   `,
    )
    if (status.done) {
      process.stdout.write('\n')
      return
    }
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, POLL_INTERVAL_MS)
      void timer
    })
  }
  process.stdout.write('\n')
  console.warn(`  Warning: poll timeout for ${conversationId}`)
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
 * Three-phase wait strategy for a single conversation after ingestion:
 * 1. Sleep 5 minutes to let auto-triggered jobs fire.
 * 2. Poll job_status until done (or timeout).
 * 3. Trigger a force-flush, then poll again to catch any remaining messages.
 */
export const waitForProcessing = async (
  baseUrl: string,
  conversationId: string,
): Promise<void> => {
  // Phase 1: initial wait
  process.stdout.write('  Phase 1: waiting 5 min for auto-triggered jobs...\n')
  await new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, 5 * 60_000)
    void timer
  })

  // Phase 2: poll until done
  process.stdout.write('  Phase 2: polling job status...\n')
  await pollUntilDone(baseUrl, conversationId, 'auto-jobs')

  // Phase 3: force-flush remainder
  process.stdout.write('  Phase 3: triggering flush...\n')
  const enqueued = await triggerFlush(baseUrl, conversationId)
  if (enqueued) {
    await pollUntilDone(baseUrl, conversationId, 'flush-job')
  }
  else {
    process.stdout.write('  Queue was already empty, no flush needed.\n')
  }
}

/**
 * Wait for all provided conversation IDs.
 * Runs sequentially to avoid hammering the server.
 */
export const waitForAll = async (
  baseUrl: string,
  conversationIds: string[],
): Promise<void> => {
  for (const id of conversationIds) {
    process.stdout.write(`Waiting for conversation ${id}...\n`)
    await waitForProcessing(baseUrl, id)
  }
}
