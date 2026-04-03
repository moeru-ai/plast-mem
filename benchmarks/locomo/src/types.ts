// LoCoMo dataset types
// mirrors the structure of locomo10.json

export interface BenchmarkComparisonMetric {
  llm_judge_delta: number
  nemori_f1_delta: number
  score_delta: number
}

export interface BenchmarkComparisonSummary {
  by_category: Record<QACategory, BenchmarkComparisonMetric>
  by_sample: Record<string, BenchmarkComparisonMetric>
  full_context: BenchmarkScoreSummary
  overall: BenchmarkComparisonMetric
  plastmem: BenchmarkScoreSummary
}

export interface BenchmarkMeta {
  base_url: string
  compare_full_context: boolean
  data_file: string
  model: string
  sample_ids: string[]
  seed?: number
  timestamp: string
  use_llm_judge: boolean
}

export interface BenchmarkOutput {
  comparison?: BenchmarkComparisonSummary
  meta: BenchmarkMeta
  variants: Partial<Record<BenchmarkVariant, BenchmarkVariantOutput>>
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

export type BenchmarkVariant = 'full_context' | 'plastmem'

export interface BenchmarkVariantOutput {
  results: QAResult[]
  stats: BenchmarkStats
}

export interface DialogTurn {
  blip_caption?: string
  compressed_text?: string
  dia_id: string // e.g. "S1:D0"
  img_file?: string
  query?: string
  search_query?: string
  speaker: string
  text: string
}

export interface LoCoMoSample {
  conversation: Record<string, DialogTurn[] | string> // session_N | session_N_date_time | session_N_observation | session_N_summary
  qa: QAPair[]
  sample_id: string
}

export interface PendingQAResult extends Omit<QAResult, 'llm_judge_score' | 'nemori_f1_score' | 'score'> {
  llm_judge_score: null | number
  nemori_f1_score: null | number
  score: null | number
}

// 1 = multi-hop, 2 = temporal, 3 = open-domain, 4 = single-hop, 5 = adversarial
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
