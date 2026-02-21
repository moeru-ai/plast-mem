import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { env } from 'node:process'

import { workspaceDir } from './workspace-dir'

const envPath = join(workspaceDir, '.env')

const envUtils = {
  parse: () => {
    if (!existsSync(envPath))
      return new Map<string, string>()
    const content = readFileSync(envPath, 'utf-8')
    return new Map<string, string>(
      content.split('\n')
        .map(line => line.trim())
        .filter(line => !line.startsWith('#') && line.includes('='))
        .map((line) => {
          const [key, ...val] = line.split('=')
          const value = val.join('=').trim().replace(/^['"]|['"]$/g, '')
          return [key.trim(), value]
        }),
    )
  },
  save: (map: Map<string, string>) => {
    const content = Array.from(map)
      .map(([k, v]) => `${k}=${/[\s"'#=]/.test(v) ? `"${v.replace(/"/g, '\\"')}"` : v}`)
      .join('\n')
    writeFileSync(envPath, content)
  },
}

export const dotenvStorage: Storage = {
  clear: () => writeFileSync(envPath, ''),
  getItem: key => env[key] ?? envUtils.parse().get(key) ?? null,
  key: index => Array.from(envUtils.parse().keys())[index] ?? null,
  length: 0,
  removeItem: (key) => {
    const env = envUtils.parse()
    if (env.delete(key))
      envUtils.save(env)
  },
  setItem: (key, value) => {
    const env = envUtils.parse()
    env.set(key, value)
    envUtils.save(env)
  },
}
