export type LongMemEvalDataset = LongMemEvalSample[]

export interface LongMemEvalDetailedResult extends LongMemEvalResult {
  answer: string
  is_correct: boolean
  is_invalid: boolean
  question_date: string
  response: string
}

export interface LongMemEvalOutput {
  item_results: LongMemEvalOutputItem[]
  meta: {
    base_url: string
    checkpoint_path?: string
    dataset: string
    model: string
    seed?: number
    timestamp: string
  }
  stats: LongMemEvalStats
}

export interface LongMemEvalOutputItem {
  item_id: string
  metrics: {
    accuracy: number
    detailed_results: LongMemEvalDetailedResult[]
    is_correct: boolean
    is_invalid: boolean
  }
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
  answer: number | string
  answer_session_ids: string[]
  haystack_dates: string[]
  haystack_session_ids: string[]
  haystack_sessions: LongMemEvalTurn[][]
  improved_answer?: number | string
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
