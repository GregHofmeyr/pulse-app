/// <reference types="node" />
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

describe('index.html', () => {
  it('turns DNS prefetching off', () => {
    const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8')
    expect(html).toMatch(/<meta\s+http-equiv="x-dns-prefetch-control"\s+content="off"\s*\/?>/)
  })
})
