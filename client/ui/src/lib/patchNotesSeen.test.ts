// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { markPatchNotesSeen, patchNotesSeen, reloadPatchNotesSeen } from './patchNotesSeen.svelte'

afterEach(() => {
  localStorage.clear()
  reloadPatchNotesSeen()
  vi.restoreAllMocks()
})

describe('patch notes seen on this device (#207)', () => {
  it('nothing seen to begin with', () => {
    expect(patchNotesSeen.id).toBeNull()
  })

  it('remembers the id of the notes opened, across a reload', () => {
    markPatchNotesSeen('2026-11-05')
    expect(patchNotesSeen.id).toBe('2026-11-05')
    reloadPatchNotesSeen()
    expect(patchNotesSeen.id).toBe('2026-11-05')
  })

  it('storage that throws is ignored: reads as unseen, and opening still clears it for now', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied') })
    reloadPatchNotesSeen()
    expect(patchNotesSeen.id).toBeNull()
    markPatchNotesSeen('x')
    expect(patchNotesSeen.id).toBe('x')
  })
})
