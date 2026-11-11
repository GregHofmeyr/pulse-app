import { describe, expect, it } from 'vitest'
import { arrange, expiresLabel, formatSize, gridColumns, isExecutable, isExpired, shownCaption, singleBox } from './attachments'
import type { Attachment } from './protocol/Attachment'

const att = (name: string, kind: Attachment['kind'], extra: Partial<Attachment> = {}): Attachment => ({
  id: name, name, content_type: 'application/octet-stream', kind, size: 1, width: null, height: null, duration_ms: null,
  expires_at: null, expired: false, ...extra,
})

describe('formatSize', () => {
  it('reads like a file manager', () => {
    expect(formatSize(512)).toBe('512 B')
    expect(formatSize(820 * 1024)).toBe('820 KB')
    expect(formatSize(1.4 * 1024 * 1024)).toBe('1.4 MB')
    expect(formatSize(50 * 1024 * 1024)).toBe('50 MB')
    expect(formatSize(2.1 * 1024 ** 3)).toBe('2.1 GB')
  })
})

describe('isExecutable', () => {
  it('flags files that can run programs, whatever the case', () => {
    for (const n of ['Setup.EXE', 'run.AppImage', 'install.msi', 'script.ps1', 'a.bat', 'mods.jar', 'go.sh']) {
      expect(isExecutable(n), n).toBe(true)
    }
    for (const n of ['notes.txt', 'photo.webp', 'world.zip', 'exe', '.bashrc-notes.md']) {
      expect(isExecutable(n), n).toBe(false)
    }
  })
})

describe('shownCaption', () => {
  it('hides the stand-in text older versions show', () => {
    expect(shownCaption({ content: '📎 report.pdf', caption_is_fallback: true })).toBe('')
    expect(shownCaption({ content: 'look', caption_is_fallback: false })).toBe('look')
    expect(shownCaption({ content: 'look' })).toBe('look')
  })
})

describe('arrange', () => {
  it('puts images in the grid, video and audio in players and the rest in cards', () => {
    const r = arrange([
      att('a.webp', 'image'), att('b.mp4', 'video'), att('c.mp3', 'audio'), att('d.pdf', 'file'),
      att('big.mp4', 'video', { expires_at: '2026-11-12T00:00:00.000Z' }),
    ])
    expect(r.images.map((a) => a.name)).toEqual(['a.webp'])
    expect(r.videos.map((a) => a.name)).toEqual(['b.mp4'])
    expect(r.audio.map((a) => a.name)).toEqual(['c.mp3'])
    expect(r.files.map((a) => a.name)).toEqual(['d.pdf', 'big.mp4'])
  })
})

describe('temporary sends', () => {
  const now = Date.parse('2026-11-11T12:00:00.000Z')
  const at = (ms: number) => new Date(now + ms).toISOString()
  it('says how long is left', () => {
    expect(expiresLabel(at(6 * 86_400_000 + 1000), now)).toBe('expires in 6 days')
    expect(expiresLabel(at(30 * 3_600_000), now)).toBe('expires tomorrow')
    expect(expiresLabel(at(5 * 3_600_000), now)).toBe('expires in 5 hours')
    expect(expiresLabel(at(20 * 60_000), now)).toBe('expires in 20 minutes')
    expect(expiresLabel(at(30_000), now)).toBe('expires in 1 minute')
  })
  it('is expired when the server says so or its time is up', () => {
    expect(isExpired({ expires_at: at(1000), expired: false }, now)).toBe(false)
    expect(isExpired({ expires_at: at(-1), expired: false }, now)).toBe(true)
    expect(isExpired({ expires_at: at(1000), expired: true }, now)).toBe(true)
    expect(isExpired({ expires_at: null, expired: false }, now)).toBe(false)
  })
})

describe('layout', () => {
  it('lays images out by count', () => {
    expect([1, 2, 4, 5, 10].map(gridColumns)).toEqual([1, 2, 2, 3, 3])
  })
  it('fits one image in 400×300 keeping its shape', () => {
    expect(singleBox(4096, 2048)).toEqual({ width: 400, height: 200 })
    expect(singleBox(1000, 3000)).toEqual({ width: 100, height: 300 })
    expect(singleBox(200, 100)).toEqual({ width: 200, height: 100 })
    expect(singleBox(null, null)).toEqual({ width: 400, height: 300 })
  })
})
