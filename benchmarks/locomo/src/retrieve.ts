import { retrieveMemory } from 'plastmem'

export const getContext = async (
  conversationId: string,
  question: string,
  baseUrl: string,
): Promise<string> => {
  const res = await retrieveMemory({
    baseUrl,
    body: {
      conversation_id: conversationId,
      episodic_limit: 10,
      query: question,
      semantic_limit: 5,
    },
    throwOnError: true,
  })
  return res.data ?? ''
}
