import { describe, expect, it } from 'vitest'
import { firstLines, previewKind, TEXT_PREVIEW_MAX } from './textPreview'
import type { Attachment } from './protocol/Attachment'

const att = (name: string, extra: Partial<Attachment> = {}): Attachment => ({
  id: 'A1', name, kind: 'file', content_type: 'application/octet-stream', size: 2048, width: null, height: null, duration_ms: null, expires_at: null, expired: false, ...extra,
})
const now = Date.parse('2026-11-17T00:00:00.000Z')

describe('previewKind (#318)', () => {
  it('previews Markdown rendered, and text, code and HTML as plain text', () => {
    expect(previewKind(att('notes.md'), now)).toBe('markdown')
    expect(previewKind(att('README.MARKDOWN'), now)).toBe('markdown')
    expect(previewKind(att('todo.txt'), now)).toBe('text')
    expect(previewKind(att('main.rs'), now)).toBe('text')
    expect(previewKind(att('page.html'), now)).toBe('text')
  })
  it('never previews other files, big ones, expired ones or media', () => {
    expect(previewKind(att('setup.exe'), now)).toBeNull()
    expect(previewKind(att('noext'), now)).toBeNull()
    expect(previewKind(att('.txt'), now)).toBeNull()
    expect(previewKind(att('big.txt', { size: TEXT_PREVIEW_MAX + 1 }), now)).toBeNull()
    expect(previewKind(att('gone.txt', { expired: true, expires_at: '2026-11-01T00:00:00.000Z' }), now)).toBeNull()
    expect(previewKind(att('pic.svg', { kind: 'image' }), now)).toBeNull()
  })
})

describe('firstLines', () => {
  it('cuts to the first lines and counts them all', () => {
    const text = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n') + '\n'
    const f = firstLines(text)
    expect(f.head.split('\n')).toHaveLength(15)
    expect(f.lines).toBe(20)
    expect(f.more).toBe(true)
    expect(firstLines('one\ntwo')).toEqual({ head: 'one\ntwo', lines: 2, more: false })
  })
})
