import { env } from 'node:process'

const BASE_URL = env.PLASTMEM_BASE_URL ?? 'http://localhost:3000'

async function post(path: string, body: unknown): Promise<string> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!res.ok)
    throw new Error(`plastmem ${path} failed: ${res.status}`)
  return res.text()
}

export const addMessage = (conversationId: string, role: 'user' | 'assistant', content: string) =>
  post('/api/v0/add_message', { conversation_id: conversationId, message: { role, content } })

export const recentMemory = (conversationId: string): Promise<string> =>
  post('/api/v0/recent_memory', { conversation_id: conversationId, limit: 10 })

export const retrieveMemory = (conversationId: string, query: string): Promise<string> =>
  post('/api/v0/retrieve_memory', { conversation_id: conversationId, query })
