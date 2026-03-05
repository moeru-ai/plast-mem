import { uuid } from '@insel-null/uuid'
import { useCallback, useState } from 'react'

import { dotenvStorage } from '../utils/dotenv-storage'

export const useConversationId = () => {
  const key = 'HARU_CONVERSATION_ID'

  const [state, setState] = useState(() => {
    const item = dotenvStorage.getItem(key)
    if (item != null) {
      return item
    }
    else {
      const initialState = uuid.v7()
      dotenvStorage.setItem(key, initialState)
      return initialState
    }
  })

  const setConversationId = useCallback((value: ReturnType<typeof uuid.v7>) => {
    setState(value)
    dotenvStorage.setItem(key, value)
  }, [])

  return [state, setConversationId] as const
}
