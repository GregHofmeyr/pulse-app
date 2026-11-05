// @vitest-environment jsdom
// The delayed "Loading…" (#173), and that every loading line in the app goes through it.
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Loading from './Loading.svelte'
import { LOADING_DELAY_MS } from '../lib/loading'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => void vi.useFakeTimers())
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('Loading', () => {
  it('shows nothing at first, then its line once the wait is over', () => {
    cmp = mount(Loading, { target: document.body, props: {} })
    flushSync()
    expect(document.querySelector('.loading')).toBeNull()
    vi.advanceTimersByTime(LOADING_DELAY_MS)
    flushSync()
    expect(document.querySelector('.loading')!.textContent).toBe('Loading…')
    expect(document.querySelector('.loading')!.getAttribute('role')).toBe('status')
  })

  it('takes its own words', () => {
    cmp = mount(Loading, { target: document.body, props: { label: 'Searching…' } })
    flushSync()
    vi.advanceTimersByTime(LOADING_DELAY_MS)
    flushSync()
    expect(document.body.textContent).toBe('Searching…')
  })
})

describe('every loading line waits', () => {
  const files: string[] = []
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) walk(join(dir, e.name))
      else if (e.name.endsWith('.svelte') && e.name !== 'Loading.svelte') files.push(join(dir, e.name))
    }
  }
  walk(resolve(__dirname, '..'))

  it('no component writes its own "Loading…" or "Searching…" paragraph (they use <Loading />)', () => {
    const offenders: string[] = []
    for (const f of files) {
      for (const line of readFileSync(f, 'utf8').split('\n')) if (/<p[^>]*>\s*(Loading|Searching)…/.test(line)) offenders.push(`${f}: ${line.trim()}`)
    }
    expect(offenders).toEqual([])
  })
})
