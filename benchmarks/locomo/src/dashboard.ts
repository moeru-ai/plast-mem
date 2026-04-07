import { stdout } from 'node:process'
import { clearLine, clearScreenDown, cursorTo, moveCursor } from 'node:readline'
import { setTimeout as sleep } from 'node:timers/promises'

const RENDER_DEBOUNCE_MS = 50
const HIDE_CURSOR = '\u001B[?25l'
const SHOW_CURSOR = '\u001B[?25h'

export class BenchmarkDashboard {
  private cursorHidden = false
  private readonly enabled: boolean
  private hasRendered = false
  private lastRenderedLines: string[] = []
  private readonly lines = new Map<string, string>()
  private renderedLineCount = 0
  private renderScheduled = false
  private readonly sampleIds: string[]
  private stopped = false

  constructor(
    sampleIds: string[],
    options?: {
      enabled?: boolean
    },
  ) {
    this.sampleIds = [...sampleIds]
    this.enabled = options?.enabled ?? stdout.isTTY

    for (const sampleId of sampleIds)
      this.lines.set(sampleId, `${sampleId}  pending`)
  }

  async flush(): Promise<void> {
    while (this.renderScheduled)
      await sleep(10)

    this.renderNow()
  }

  setLine(sampleId: string, line: string): void {
    this.lines.set(sampleId, line)
    this.scheduleRender()
  }

  async stop(): Promise<void> {
    this.stopped = true
    await this.flush()
    this.restoreCursor()
  }

  private buildLines(): string[] {
    return this.sampleIds.map(sampleId =>
      this.fitLine(this.lines.get(sampleId) ?? `${sampleId}  pending`),
    )
  }

  private fitLine(line: string): string {
    const columns = stdout.columns
    if (columns == null || columns <= 1)
      return line

    const maxWidth = columns
    if (line.length <= maxWidth)
      return line

    return `${line.slice(0, Math.max(0, maxWidth - 1))}…`
  }

  private hideCursor(): void {
    if (this.cursorHidden)
      return

    stdout.write(HIDE_CURSOR)
    this.cursorHidden = true
  }

  private renderNow(): void {
    if (!this.enabled)
      return

    this.hideCursor()
    const contentLines = this.buildLines()

    if (!this.hasRendered) {
      stdout.write(`${contentLines.join('\n')}\n`)
      this.lastRenderedLines = [...contentLines]
      this.renderedLineCount = contentLines.length
      this.hasRendered = true
      return
    }

    if (contentLines.length !== this.renderedLineCount) {
      moveCursor(stdout, 0, -this.renderedLineCount)
      cursorTo(stdout, 0)
      clearScreenDown(stdout)
      stdout.write(`${contentLines.join('\n')}\n`)
      this.lastRenderedLines = [...contentLines]
      this.renderedLineCount = contentLines.length
      return
    }

    for (let index = 0; index < contentLines.length; index += 1) {
      const nextLine = contentLines[index]
      if (this.lastRenderedLines[index] === nextLine)
        continue

      const offset = this.renderedLineCount - index
      moveCursor(stdout, 0, -offset)
      cursorTo(stdout, 0)
      clearLine(stdout, 0)
      stdout.write(nextLine)
      moveCursor(stdout, 0, offset)
      cursorTo(stdout, 0)
      this.lastRenderedLines[index] = nextLine
    }
  }

  private restoreCursor(): void {
    if (!this.cursorHidden)
      return

    stdout.write(SHOW_CURSOR)
    this.cursorHidden = false
  }

  private scheduleRender(): void {
    if (this.stopped || this.renderScheduled)
      return

    this.renderScheduled = true
    void sleep(RENDER_DEBOUNCE_MS).then(() => {
      this.renderScheduled = false
      this.renderNow()
    })
  }
}

export const renderProgressBar = (
  done: number,
  total: number,
  width: number = 12,
): string => {
  if (total <= 0)
    return '[............]'

  const normalizedDone = Math.max(0, Math.min(done, total))
  const filled = Math.round((normalizedDone / total) * width)
  const clampedFilled = Math.max(0, Math.min(filled, width))
  return `[${'#'.repeat(clampedFilled)}${'.'.repeat(width - clampedFilled)}]`
}
