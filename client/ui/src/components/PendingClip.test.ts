// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => `asset://${s}` }))

import PendingClip from './PendingClip.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { reactiveProps } from '../lib/testProps.svelte'
import { testMe, testReady } from '../lib/testReady'
import type { ClipJob } from '../lib/tauri'

const ready = testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'clips', position: 0 }], {
  members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
  games: [{ id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
})

const job = (over: Partial<ClipJob> = {}): ClipJob => ({
  id: 'J1', channel_id: 'C1', stage: 'preparing', progress: 0, error: null, caption: 'what a shot', game_id: 'G1',
  kept_ms: 12_000, thumb_path: null, message_id: null, ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const card = () => document.querySelector<HTMLElement>('.pending-clip')!
const button = (text: string) => [...card().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)

function show(j: ClipJob) {
  const p = reactiveProps({ job: j })
  cmp = mount(PendingClip, { target: document.body, props: p })
  flushSync()
  return p
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
  app.state = applyReady(emptyState(), ready)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('PendingClip', () => {
  it('shows preparing then uploading percentages', () => {
    const p = show(job({ stage: 'preparing', progress: 0.374 }))
    expect(card().textContent).toContain('Preparing… 37%')
    expect(card().textContent).toContain('what a shot')
    expect(card().textContent).toContain('Apex')
    expect(card().querySelector('img')).toBeNull() // no poster yet
    expect(button('Cancel')).toBeTruthy()

    p.job = job({ stage: 'uploading', progress: 0.5, thumb_path: '/data/jobs/J1/thumb.jpg' })
    flushSync()
    expect(card().textContent).toContain('Uploading… 50%')
    expect(card().textContent).not.toContain('Preparing')
    expect(card().querySelector('img')?.getAttribute('src')).toBe('asset:///data/jobs/J1/thumb.jpg')

    p.job = job({ stage: 'posting', progress: 1, thumb_path: '/data/jobs/J1/thumb.jpg' })
    flushSync()
    expect(card().textContent).toContain('Posting…')
    expect(button('Cancel')).toBeTruthy()
  })

  it('failed shows retry and the reason', async () => {
    const p = show(job({ stage: 'failed', error: "That's 20 clips this hour. Try again later." }))
    expect(card().textContent).toContain('Upload failed')
    expect(card().textContent).toContain("That's 20 clips this hour. Try again later.")
    expect(button('Cancel')).toBeTruthy()
    button('Retry')!.click()
    flushSync()
    expect(calls('retry_clip')).toEqual([['retry_clip', { id: 'J1' }]])
    await settle()

    // no words of the server's: just the retry line
    p.job = job({ stage: 'failed', error: null })
    flushSync()
    expect(card().querySelector('.why')).toBeNull()
    expect(button('Retry')).toBeTruthy()

    // a retry the server refuses says so in its own words
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'retry_clip') throw 'Clips storage isn\'t set up on this server.'
      return undefined
    })
    button('Retry')!.click()
    await settle()
    flushSync()
    expect(card().textContent).toContain("Clips storage isn't set up on this server.")
  })

  it('cancel calls cancel_clip', async () => {
    let finish: () => void = () => {}
    invoke.mockImplementation(async (cmd) => (cmd === 'cancel_clip' ? new Promise<void>((r) => (finish = r)) : undefined))
    show(job({ stage: 'posting', progress: 1 }))
    button('Cancel')!.click()
    flushSync()
    expect(calls('cancel_clip')).toEqual([['cancel_clip', { id: 'J1' }]])
    // one press: a second can't race the first while it posts
    expect(button('Cancel')!.disabled).toBe(true)
    button('Cancel')!.click()
    flushSync()
    expect(calls('cancel_clip')).toHaveLength(1)
    finish()
    await settle()
  })

  it('done shows no cancel', () => {
    show(job({ stage: 'done', progress: 1, message_id: 'M1' }))
    expect(button('Cancel')).toBeUndefined()
  })
})
