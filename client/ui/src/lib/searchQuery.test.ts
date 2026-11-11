import { describe, expect, it } from 'vitest'
import { activeTerm, chipText, filterRows, HAS_VALUES, hasSearchableWords, parseText, replaceTerm, toRequest, type Chip } from './searchQuery'

describe('parseText', () => {
  it('keeps plain words and quoted phrases as the words', () => {
    expect(parseText('pizza "big clip" tonight')).toEqual({ words: 'pizza "big clip" tonight', filters: [] })
  })
  it('takes filters out, in any order', () => {
    expect(parseText('from:sam pizza has:link in:general')).toEqual({
      words: 'pizza',
      filters: [{ key: 'from', value: 'sam' }, { key: 'has', value: 'link' }, { key: 'in', value: 'general' }],
    })
  })
  it('treats unknown filters and a filter inside quotes as words', () => {
    expect(parseText('foo:bar "from:sam" x')).toEqual({ words: 'foo:bar "from:sam" x', filters: [] })
  })
  it('leaves a filter with no value as words', () => {
    expect(parseText('from: pizza')).toEqual({ words: 'from: pizza', filters: [] })
  })
  it('accepts a leading # on in: and a leading @ on a person', () => {
    expect(parseText('in:#general').filters).toEqual([{ key: 'in', value: 'general' }])
    expect(parseText('from:@sam mentions:@alex').filters).toEqual([{ key: 'from', value: 'sam' }, { key: 'mentions', value: 'alex' }])
    expect(activeTerm('from:@sa', 8).value).toBe('sa')
  })
})

describe('activeTerm', () => {
  it('is empty at the start of a new term', () => {
    expect(activeTerm('', 0)).toEqual({ start: 0, filter: null, value: '', token: '' })
    expect(activeTerm('pizza ', 6)).toEqual({ start: 6, filter: null, value: '', token: '' })
  })
  it('names the filter whose value is being typed', () => {
    expect(activeTerm('pizza from:sa', 13)).toEqual({ start: 6, filter: 'from', value: 'sa', token: 'from:sa' })
    expect(activeTerm('MENTIONS:', 9)).toEqual({ start: 0, filter: 'mentions', value: '', token: 'MENTIONS:' })
    expect(activeTerm('in:#gen', 7).value).toBe('gen')
  })
  it('is a plain word otherwise', () => {
    expect(activeTerm('piz', 3)).toEqual({ start: 0, filter: null, value: '', token: 'piz' })
  })
})

describe('hasSearchableWords', () => {
  it('needs a letter or a digit: emoji and punctuation alone find nothing', () => {
    expect(hasSearchableWords('pizza')).toBe(true)
    expect(hasSearchableWords('Zoë 42')).toBe(true)
    expect(hasSearchableWords('🔥 ?! ""')).toBe(false)
    expect(hasSearchableWords('  ')).toBe(false)
  })
})

describe('replaceTerm', () => {
  it('puts the caret after what it inserts', () => {
    expect(replaceTerm('pizza fr', 6, 8, 'from:')).toEqual({ text: 'pizza from:', caret: 11 })
    expect(replaceTerm('from:sa pizza', 0, 7, '')).toEqual({ text: 'pizza', caret: 0 })
  })
})

describe('filterRows', () => {
  it('offers in: only in a server', () => {
    expect(filterRows('', true).map((r) => r.key)).toEqual(['from', 'mentions', 'has', 'in'])
    expect(filterRows('', false).map((r) => r.key)).toEqual(['from', 'mentions', 'has'])
  })
  it('narrows to filters that start with what is typed', () => {
    expect(filterRows('m', true).map((r) => r.key)).toEqual(['mentions'])
    expect(filterRows('pizza', true)).toEqual([])
  })
})

describe('toRequest and chips', () => {
  const chips: Chip[] = [
    { key: 'from', id: 'U2', label: 'sam' },
    { key: 'has', value: 'clip', label: 'clip' },
    { key: 'has', value: 'link', label: 'link' },
    { key: 'in', id: 'C1', label: 'general' },
    { key: 'mentions', id: 'U1', label: 'alex' },
  ]
  it('turns words and chips into structured parameters', () => {
    expect(toRequest(' pizza ', chips, 'M9')).toEqual({ q: 'pizza', from: 'U2', in_channel: 'C1', has: ['clip', 'link'], mentions: 'U1', cursor: 'M9' })
    expect(toRequest('x', [])).toEqual({ q: 'x', from: null, in_channel: null, has: [], mentions: null, cursor: null })
  })
  it('writes chips as their filter syntax', () => {
    expect(chips.map(chipText)).toEqual(['from: sam', 'has: clip', 'has: link', 'in: #general', 'mentions: alex'])
  })
})

describe('has: files (#57)', () => {
  it('parses media and file, and offers them', () => {
    expect(parseText('has:media has:file cat').filters).toEqual([{ key: 'has', value: 'media' }, { key: 'has', value: 'file' }])
    expect(HAS_VALUES.map((h) => h.value)).toEqual(['link', 'media', 'file', 'clip', 'quote'])
  })
})
