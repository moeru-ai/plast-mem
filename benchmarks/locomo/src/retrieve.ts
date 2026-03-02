import { contextPreRetrieve } from 'plastmem'

export const getContext = async (
  conversationId: string,
  question: string,
  baseUrl: string,
): Promise<string> => {
  const res = await contextPreRetrieve({
    baseUrl,
    body: {
      conversation_id: conversationId,
      query: question,
    },
    throwOnError: true,
  })
  return res.data ?? ''
}
