// LoCoMo dataset types
// mirrors the structure of locomo10.json

export interface BenchmarkOutput {
  meta: {
    base_url: string
    data_file: string
    model: string
    timestamp: string
  }
  results: QAResult[]
  stats: BenchmarkStats
}

export interface BenchmarkScoreSummary {
  by_category: Record<QACategory, number>
  by_category_count: Record<QACategory, number>
  by_category_llm: Record<QACategory, number>
  by_category_nemori_f1: Record<QACategory, number>
  overall: number
  overall_llm: number
  overall_nemori_f1: number
  total: number
}

export interface BenchmarkStats {
  by_sample: Record<string, BenchmarkScoreSummary>
  overall: BenchmarkScoreSummary
}
// 1 = multi-hop, 2 = temporal, 3 = open-domain, 4 = single-hop, 5 = adversarial

export interface DialogTurn {
  blip_caption?: string
  compressed_text?: string
  dia_id: string // e.g. "S1:D0"
  img_file?: string
  search_query?: string
  speaker: string
  text: string
}

export interface LoCoMoSample {
  conversation: Record<string, DialogTurn[] | string> // session_N | session_N_date_time | session_N_observation | session_N_summary
  qa: QAPair[]
  sample_id: string
}

export type QACategory = 1 | 2 | 3 | 4 | 5

export interface QAPair {
  adversarial_answer: null | string
  answer: number | string
  category: QACategory
  evidence: string[] // dia_ids containing the answer
  question: string
}

// Final result record written to the output JSON file
export interface QAResult {
  category: QACategory
  context_retrieved: string
  evidence: string[]
  gold_answer: number | string
  llm_judge_score: number
  nemori_f1_score: number
  prediction: string
  question: string
  sample_id: string
  score: number
}
