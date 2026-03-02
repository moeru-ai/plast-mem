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

export interface BenchmarkStats {
  by_category: Record<QACategory, number>
  by_category_count: Record<QACategory, number>
  overall: number
  total: number
}
// 1 = multi-hop, 2 = single-hop, 3 = temporal, 4 = open-domain, 5 = adversarial

export interface DialogTurn {
  blip_caption?: string
  clean_text: string
  compressed_text?: string
  dia_id: string // e.g. "S1:D0"
  img_file?: string
  search_query?: string
  speaker: string
}

export interface LoCoMoSample {
  conversation: Record<string, DialogTurn[] | string> // session_N | session_N_date_time | session_N_observation | session_N_summary
  qa: QAPair[]
  sample_id: string
}

export type QACategory = 1 | 2 | 3 | 4 | 5

export interface QAPair {
  adversarial_answer: null | string
  answer: string
  category: QACategory
  evidence: string[] // dia_ids containing the answer
  question: string
}

// Result record written to the output JSON file
export interface QAResult {
  category: QACategory
  context_retrieved: string
  evidence: string[]
  gold_answer: string
  prediction: string
  question: string
  sample_id: string
  score: number
}
