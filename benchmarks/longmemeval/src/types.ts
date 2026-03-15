export type LongMemEvalDataset = LongMemEvalSample[]

export type LongMemEvalQuestionType
  = | 'knowledge-update'
    | 'multi-session'
    | 'single-session-assistant'
    | 'single-session-preference'
    | 'single-session-user'
    | 'temporal-reasoning'

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

export interface LongMemEvalTurn {
  content: string
  has_answer?: boolean
  role: 'assistant' | 'user'
}
