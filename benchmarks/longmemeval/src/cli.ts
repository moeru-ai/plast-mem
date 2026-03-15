import process from 'node:process'

import c from 'tinyrainbow'

import * as p from '@clack/prompts'

import { name } from '../package.json'
import { checkDataset, downloadDataset, loadDataset } from './utils/dataset'

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

  const firstSample = dataset[0]
  if (firstSample != null) {
    const keys = Object.keys(firstSample)
    p.log.info(`first sample keys: ${keys.join(', ')}`)
  }

  p.outro('Dataset is ready. Next step is wiring ingest / retrieve / eval around this schema.')
}

// eslint-disable-next-line @masknet/no-top-level
main().catch(console.error)
