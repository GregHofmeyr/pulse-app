// Downloads in progress (#205): what each one shows, a second click cancels, Saved goes quiet after a moment, and
// the polite announcements (start and finish, never every percent).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { clipKey, downloads, fileKey, imageKey, SAVED_MS } from './downloads.svelte'

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}
/** A save the test finishes: `finish(true)` saved, `finish(false)` cancelled, `fail(text)` failed. */
function held() {
  let finish: (v: boolean) => void = () => {}
  let fail: (e: unknown) => void = () => {}
  const save = vi.fn(() => new Promise<boolean>((res, rej) => ((finish = res), (fail = rej))))
  return { save, finish: (v: boolean) => finish(v), fail: (e: unknown) => fail(e) }
}

beforeEach(() => {
  invoke.mockReset()
  vi.useFakeTimers()
})
afterEach(() => {
  downloads.reset()
  vi.useRealTimers()
})

describe('downloads', () => {
  it('keys say what is saved', () => {
    expect(clipKey('M1')).toBe('clip:M1')
    expect(imageKey('abc')).toBe('image:abc')
    expect(fileKey('A1')).toBe('file:A1')
  })

  it('shows nothing while the Save dialog is open, then the share the core reports', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    expect(downloads.view('clip:M1')).toBeNull()
    downloads.progress({ key: 'clip:M1', received: 0, total: 1000 })
    expect(downloads.view('clip:M1')).toEqual({ phase: 'downloading', fraction: 0 })
    downloads.progress({ key: 'clip:M1', received: 420, total: 1000 })
    expect(downloads.view('clip:M1')).toEqual({ phase: 'downloading', fraction: 0.42 })
    s.finish(true)
    await settle()
  })

  it('an unknown size is indeterminate', () => {
    void downloads.toggle('file:A1', 'big.zip', held().save, vi.fn())
    downloads.progress({ key: 'file:A1', received: 5000, total: null })
    expect(downloads.view('file:A1')).toEqual({ phase: 'downloading', fraction: null })
  })

  it('ignores progress for a download that is not running', () => {
    downloads.progress({ key: 'clip:M9', received: 1, total: 2 })
    expect(downloads.view('clip:M9')).toBeNull()
  })

  it('a finished download says Saved for a moment, then goes', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    downloads.progress({ key: 'clip:M1', received: 0, total: 10 })
    s.finish(true)
    await settle()
    expect(downloads.view('clip:M1')).toEqual({ phase: 'saved' })
    // a late progress event changes nothing
    downloads.progress({ key: 'clip:M1', received: 10, total: 10 })
    expect(downloads.view('clip:M1')).toEqual({ phase: 'saved' })
    vi.advanceTimersByTime(SAVED_MS)
    expect(downloads.view('clip:M1')).toBeNull()
  })

  it('a second click while it downloads cancels it, quietly', async () => {
    const s = held()
    const onError = vi.fn()
    void downloads.toggle('clip:M1', 'clip', s.save, onError)
    downloads.progress({ key: 'clip:M1', received: 5, total: 10 })
    void downloads.toggle('clip:M1', 'clip', s.save, onError)
    expect(invoke).toHaveBeenCalledWith('cancel_download', { key: 'clip:M1' })
    expect(s.save).toHaveBeenCalledTimes(1)
    s.finish(false) // the core answers false once it has stopped
    await settle()
    expect(downloads.view('clip:M1')).toBeNull()
    expect(onError).not.toHaveBeenCalled()
  })

  it('a second click while its Save dialog is open does nothing', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    expect(s.save).toHaveBeenCalledTimes(1)
    expect(invoke).not.toHaveBeenCalled()
    s.finish(false)
    await settle()
    // free again
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    expect(s.save).toHaveBeenCalledTimes(2)
    s.finish(false)
    await settle()
  })

  it('a failure clears it and hands the error to its owner', async () => {
    const s = held()
    const onError = vi.fn()
    void downloads.toggle('clip:M1', 'clip', s.save, onError)
    downloads.progress({ key: 'clip:M1', received: 5, total: 10 })
    s.fail('the download stopped moving')
    await settle()
    expect(downloads.view('clip:M1')).toBeNull()
    expect(onError).toHaveBeenCalledWith('the download stopped moving')
    expect(downloads.announcement).toBe('')
  })

  it('announces the start and the finish, not every percent', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    expect(downloads.announcement).toBe('')
    downloads.progress({ key: 'clip:M1', received: 0, total: 10 })
    expect(downloads.announcement).toBe('Downloading clip')
    downloads.progress({ key: 'clip:M1', received: 5, total: 10 })
    expect(downloads.announcement).toBe('Downloading clip')
    s.finish(true)
    await settle()
    expect(downloads.announcement).toBe('Saved clip')
  })

  it('a cancel is announced as stopped', async () => {
    const s = held()
    void downloads.toggle('file:A1', 'notes.txt', s.save, vi.fn())
    downloads.progress({ key: 'file:A1', received: 1, total: 10 })
    downloads.cancel('file:A1')
    s.finish(false)
    await settle()
    expect(downloads.announcement).toBe('Stopped downloading notes.txt')
  })

  it('a dialog closed without saving announces nothing', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    s.finish(false)
    await settle()
    expect(downloads.announcement).toBe('')
  })
})
