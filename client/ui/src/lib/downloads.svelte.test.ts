// Downloads in progress (#205): what each one shows, a second click cancels, Saved goes quiet after a moment, and
// the polite announcements (start and finish, never every percent).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { clipKey, downloads, fileKey, imageKey, SAVED_MS } from './downloads.svelte'
import { microtasks } from './testSettle'

const SAVED_TO = '/home/alex/Videos/ace.mp4'
/** A save the test finishes: `finish(true)` saved (to SAVED_TO, or the path given), `finish(false)` cancelled, `fail(text)` failed. */
function held() {
  let finish: (v: string | null) => void = () => {}
  let fail: (e: unknown) => void = () => {}
  const save = vi.fn(() => new Promise<string | null>((res, rej) => ((finish = res), (fail = rej))))
  const answer = (v: boolean | string) => (v === true ? SAVED_TO : v === false ? null : v)
  return { save, finish: (v: boolean | string) => finish(answer(v)), fail: (e: unknown) => fail(e) }
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
    await microtasks()
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
    await microtasks()
    expect(downloads.view('clip:M1')).toEqual({ phase: 'saved', path: SAVED_TO })
    // a late progress event changes nothing
    downloads.progress({ key: 'clip:M1', received: 10, total: 10 })
    expect(downloads.view('clip:M1')).toEqual({ phase: 'saved', path: SAVED_TO })
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
    await microtasks()
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
    await microtasks()
    // free again
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    expect(s.save).toHaveBeenCalledTimes(2)
    s.finish(false)
    await microtasks()
  })

  it('a failure clears it and hands the error to its owner', async () => {
    const s = held()
    const onError = vi.fn()
    void downloads.toggle('clip:M1', 'clip', s.save, onError)
    downloads.progress({ key: 'clip:M1', received: 5, total: 10 })
    s.fail('the download stopped moving')
    await microtasks()
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
    await microtasks()
    expect(downloads.announcement).toBe('Saved clip')
  })

  it('a cancel is announced as stopped', async () => {
    const s = held()
    void downloads.toggle('file:A1', 'notes.txt', s.save, vi.fn())
    downloads.progress({ key: 'file:A1', received: 1, total: 10 })
    downloads.cancel('file:A1')
    s.finish(false)
    await microtasks()
    expect(downloads.announcement).toBe('Stopped downloading notes.txt')
  })

  it('Show in folder (#333) reveals the file it saved', async () => {
    const s = held()
    void downloads.toggle('file:A1', 'notes.txt', s.save, vi.fn())
    s.finish('/home/sam/Downloads/notes.txt')
    await microtasks()
    const onError = vi.fn()
    await downloads.reveal('file:A1', onError)
    expect(invoke).toHaveBeenCalledWith('reveal_path', { path: '/home/sam/Downloads/notes.txt' })
    expect(onError).not.toHaveBeenCalled()
  })

  it('Show in folder says why it could not', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    s.finish(true)
    await microtasks()
    invoke.mockRejectedValueOnce("Couldn't open that folder. The file may have moved.")
    const onError = vi.fn()
    await downloads.reveal('clip:M1', onError)
    expect(onError).toHaveBeenCalledWith("Couldn't open that folder. The file may have moved.")
  })

  it('Show in folder does nothing once Saved has gone', async () => {
    await downloads.reveal('clip:M9', vi.fn())
    expect(invoke).not.toHaveBeenCalled()
  })

  it('Saved stays while the pointer or focus is on Show in folder, then goes a moment after', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    s.finish(true)
    await microtasks()
    downloads.hold('clip:M1')
    vi.advanceTimersByTime(SAVED_MS * 4)
    expect(downloads.view('clip:M1')).toEqual({ phase: 'saved', path: SAVED_TO })
    downloads.release('clip:M1')
    vi.advanceTimersByTime(SAVED_MS - 1)
    expect(downloads.view('clip:M1')?.phase).toBe('saved')
    vi.advanceTimersByTime(1)
    expect(downloads.view('clip:M1')).toBeNull()
  })

  it('a dialog closed without saving announces nothing', async () => {
    const s = held()
    void downloads.toggle('clip:M1', 'clip', s.save, vi.fn())
    s.finish(false)
    await microtasks()
    expect(downloads.announcement).toBe('')
  })
})
