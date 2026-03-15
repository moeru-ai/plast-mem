export type LongMemEvalDataset = LongMemEvalSample[]

export interface LongMemEvalOutput {
  meta: {
    base_url: string
    model: string
    timestamp: string
  }
  results: LongMemEvalResult[]
  stats: LongMemEvalStats
}

export type LongMemEvalQuestionType
  = | 'knowledge-update'
    | 'multi-session'
    | 'single-session-assistant'
    | 'single-session-preference'
    | 'single-session-user'
    | 'temporal-reasoning'

export interface LongMemEvalResult {
  context: string
  conversation_id: string
  gold_answer: string
  prediction: string
  question: string
  question_id: string
  question_type: LongMemEvalQuestionType
  score: 0 | 1
  verdict: string
}

export interface LongMemEvalSample {
  answer: string
  answer_session_ids: string[]
  haystack_dates: string[]
  haystack_session_ids: string[]
  haystack_sessions: LongMemEvalTurn[][]
  improved_answer?: string
  improved_question?: string
  improvement_note?: string
  question: string
  question_date: string
  question_id: string
  question_type: LongMemEvalQuestionType
  requires_retry?: boolean
}

export interface LongMemEvalStats {
  by_question_type: Record<LongMemEvalQuestionType, number>
  by_question_type_count: Record<LongMemEvalQuestionType, number>
  overall: number
  total: number
}

export interface LongMemEvalTurn {
  content: string
  has_answer?: boolean
  role: 'assistant' | 'user'
}
