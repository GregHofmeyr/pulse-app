/// <reference types="node" />
// `app.state` is held raw (`$state.raw`, store.svelte.ts): writing into it in place neither
// re-renders nor stays private to one state, so the only allowed change is reassigning
// `app.state` with the reducer's output. This scan is a tripwire for the obvious ways to break that.
import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const src = new URL('../', import.meta.url)
const sources = (readdirSync(src, { recursive: true }) as string[])
  .filter((f) => /\.(ts|svelte)$/.test(f) && !/\.test\.ts$/.test(f))
  .map((f) => ({ f, text: readFileSync(new URL(f, src), 'utf8') }))

// `app.state.x = …`, `app.state.x[k] += …`, `app.state.x++` (but not `app.state = …` or `==`)
const ASSIGN = /app\.state(\.[\w$]+|\[[^\]]+\])+\s*(=(?![=>])|\+\+|--|[-+*/]=)/
// `app.state.x.push(…)` and the other in-place array, Map and Set methods
const MUTATE = /app\.state(\.[\w$]+|\[[^\]]+\])*\.(push|pop|shift|unshift|splice|sort|reverse|fill|set|add|delete|clear)\(/

describe('app.state is never written in place', () => {
  it('scans some source', () => {
    expect(sources.some(({ f }) => f.endsWith('store.svelte.ts'))).toBe(true)
  })

  it('the patterns catch an in-place write', () => {
    expect(ASSIGN.test('app.state.conn = "x"')).toBe(true)
    expect(ASSIGN.test("app.state.messages['C1'] = []")).toBe(true)
    expect(MUTATE.test('app.state.servers.push(s)')).toBe(true)
    expect(ASSIGN.test('app.state = next')).toBe(false)
    expect(ASSIGN.test("app.state.conn === 'live'")).toBe(false)
  })

  it('no source file assigns into app.state or mutates a slice of it', () => {
    const hits = sources.flatMap(({ f, text }) =>
      text.split('\n').flatMap((line, i) => (ASSIGN.test(line) || MUTATE.test(line) ? [`${f}:${i + 1}: ${line.trim()}`] : [])),
    )
    expect(hits).toEqual([])
  })
})
