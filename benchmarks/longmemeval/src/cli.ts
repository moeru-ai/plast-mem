import type { LongMemEvalDataset, LongMemEvalQuestionType } from './types'

import process from 'node:process'

import c from 'tinyrainbow'

import * as p from '@clack/prompts'

import { name } from '../package.json'
import { checkDataset, downloadDataset, loadDataset } from './utils/dataset'

const buildQuestionTypeCounts = (dataset: LongMemEvalDataset): Record<LongMemEvalQuestionType, number> => {
  const counts: Record<LongMemEvalQuestionType, number> = {
    'knowledge-update': 0,
    'multi-session': 0,
    'single-session-assistant': 0,
    'single-session-preference': 0,
    'single-session-user': 0,
    'temporal-reasoning': 0,
  }

  for (const sample of dataset)
    counts[sample.question_type] += 1

  return counts
}

const summarizeQuestionTypes = (dataset: LongMemEvalDataset): string =>
  Object.entries(buildQuestionTypeCounts(dataset))
    .filter(([, count]) => count > 0)
    .map(([type, count]) => `${type}=${count}`)
    .join(', ')

const promptQuestionTypes = async (dataset: LongMemEvalDataset): Promise<LongMemEvalQuestionType[]> => {
  const counts = buildQuestionTypeCounts(dataset)
  const selected = await p.multiselect({
    initialValues: Object.entries(counts)
      .filter(([, count]) => count > 0)
      .map(([type]) => type),
    message: 'Choose question types to run',
    options: Object.entries(counts)
      .filter(([, count]) => count > 0)
      .map(([type, count]) => ({
        hint: `${count} samples`,
        label: type,
        value: type,
      })),
    required: false,
  })

  if (p.isCancel(selected)) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  return selected as LongMemEvalQuestionType[]
}

const main = async () => {
  console.clear()
  console.log('')

  p.intro(c.bgCyan(c.black(` ${name} `)))

  let path = await checkDataset()
  if (path == null) {
    const confirmDownload = await p.confirm({
      message: 'The LongMemEval-S dataset was not found. Would you like to download it?',
    })

    if (confirmDownload === true) {
      const spinner = p.spinner()
      spinner.start('Downloading via huggingface...')
      try {
        const downloadedPath = await downloadDataset()
        spinner.stop('Downloaded')
        path = downloadedPath
      }
      catch (err) {
        spinner.error(err instanceof Error ? err.message : undefined)
        p.cancel('Operation cancelled.')
        process.exit(0)
      }
    }
    else {
      p.cancel('Operation cancelled.')
      process.exit(0)
    }
  }

  const dataset = await loadDataset(path)

  p.log.info(`file path: ${path}`)
  p.log.info(`loaded samples: ${dataset.length}`)
  p.log.info(`question types: ${summarizeQuestionTypes(dataset)}`)

  const selectedQuestionTypes = await promptQuestionTypes(dataset)
  const filteredDataset = dataset.filter(sample => selectedQuestionTypes.includes(sample.question_type))

  p.note([
    `selected question types: ${selectedQuestionTypes.join(', ')}`,
    `filtered samples: ${filteredDataset.length}/${dataset.length}`,
    `filtered type counts: ${summarizeQuestionTypes(filteredDataset)}`,
  ].join('\n'), 'Run Summary')

  const firstSample = filteredDataset[0]
  if (firstSample != null) {
    const sessionCount = firstSample.haystack_sessions.length
    const turnCount = firstSample.haystack_sessions.reduce((total, session) => total + session.length, 0)
    p.log.info(`first sample: ${firstSample.question_id} (${firstSample.question_type})`)
    p.log.info(`first sample sessions: ${sessionCount}`)
    p.log.info(`first sample turns: ${turnCount}`)
    p.log.info(`first sample answer sessions: ${firstSample.answer_session_ids.length}`)
    p.log.info(`first question: ${firstSample.question}`)
  }

  p.outro('Dataset is ready. Next step is wiring ingest / retrieve / eval around this schema.')
}

// eslint-disable-next-line @masknet/no-top-level
main().catch(console.error)
