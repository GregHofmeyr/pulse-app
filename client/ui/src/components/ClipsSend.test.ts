// @vitest-environment jsdom
// Sharing a clip from a clips channel: dropping a file or picking one opens the editor; the clips being sent
// show as pending cards at the newest end of the feed.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { ClipJob, ClipSource } from '../lib/tauri'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => `asset://${s}` }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
type DragDrop = { type: 'enter'; paths: string[] } | { type: 'over' } | { type: 'drop'; paths: string[] } | { type: 'leave' }
const drag = vi.hoisted(() => ({ handler: null as ((e: { payload: unknown }) => void) | null }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (fn: (e: { payload: unknown }) => void) => {
      drag.handler = fn
      return () => {
        if (drag.handler === fn) drag.handler = null
      }
    },
  }),
}))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ClipsChannel from './ClipsChannel.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState, setClipJob } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const ready = (over = {}) =>
  testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    games: [{ id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
    ...over,
  })
const clip = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: `caption ${id}`, reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null,
  clip: { thumb_hash: id.toLowerCase().padEnd(64, '0'), duration_ms: 65000, width: 1280, height: 720, size: 1000, game_id: 'G1', reply_count: 0, last_reply_id: null, replies_seen: null },
})
const source: ClipSource = {
  id: 'SRC1', path: '/home/alex/Videos/Apex/replay.mp4', file_name: 'replay.mp4', size: 300_000_000, duration_ms: 41_000,
  width: 2560, height: 1440, fps: 60, video_codec: 'h264', game_guess: 'apex', preview: 'direct',
}
const job = (id: string, channel_id: string, over: Partial<ClipJob> = {}): ClipJob => ({
  id, channel_id, stage: 'uploading', progress: 0.25, error: null, caption: `sending ${id}`, game_id: null, kept_ms: 9000,
  thumb_path: null, message_id: null, ...over,
})

let support = { can_send: true, reason: null as string | null }
let picked: string | null = null
let opens: (() => Promise<ClipSource>) = async () => source
let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!
const editor = () => document.querySelector('[role="dialog"][aria-label="Share a clip"]')
const chan = () => document.querySelector<HTMLElement>('.chan')!

async function show() {
  cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
  flushSync()
  await settle()
  flushSync()
}
async function dragEvent(e: DragDrop) {
  drag.handler!({ payload: { ...e, position: { x: 10, y: 10 } } })
  flushSync()
  await settle()
  await settle()
  flushSync()
}

beforeEach(() => {
  support = { can_send: true, reason: null }
  picked = null
  opens = async () => source
  drag.handler = null
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'list_clips') return [clip('M2'), clip('M1')]
    if (cmd === 'clip_facets') return { games: [], authors: [] }
    if (cmd === 'clip_support') return support
    if (cmd === 'pick_clip_file') return picked
    if (cmd === 'open_clip') return opens()
    if (cmd === 'clip_filmstrip') return { path: '/tmp/film.jpg', tiles: 24 }
    return undefined
  })
  app.state = applyReady(emptyState(), ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ClipsSend', () => {
  it('dropping a video opens the editor', async () => {
    await show()
    expect(drag.handler).not.toBeNull()
    await dragEvent({ type: 'enter', paths: ['/home/alex/Videos/Apex/replay.mp4'] })
    await dragEvent({ type: 'over' })
    expect(chan().querySelector('.drop-zone')).not.toBeNull() // highlighted while a file is over it
    await dragEvent({ type: 'drop', paths: ['/home/alex/Videos/Apex/replay.mp4'] })
    expect(chan().querySelector('.drop-zone')).toBeNull()
    expect(calls('clip_support')).toHaveLength(1)
    expect(calls('open_clip')).toEqual([['open_clip', { path: '/home/alex/Videos/Apex/replay.mp4' }]])
    expect(editor()).not.toBeNull()
    expect(editor()!.textContent).toContain('replay.mp4')

    // a drag that leaves takes the highlight with it
    await dragEvent({ type: 'over' })
    await dragEvent({ type: 'leave' })
    expect(chan().querySelector('.drop-zone')).toBeNull()

    // the editor's Close puts it away
    editor()!.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click()
    await settle()
    flushSync()
    expect(editor()).toBeNull()

    // leaving the channel stops listening
    unmount(cmp!)
    cmp = undefined
    await settle()
    expect(drag.handler).toBeNull()
  })

  it('two files open the first and say one at a time', async () => {
    await show()
    await dragEvent({ type: 'drop', paths: ['/v/first.mp4', '/v/second.mp4'] })
    expect(calls('open_clip')).toEqual([['open_clip', { path: '/v/first.mp4' }]])
    expect(document.body.textContent).toContain('One clip at a time.')
    expect(editor()).not.toBeNull()
  })

  it('share picks a file and opens it', async () => {
    picked = '/v/picked.mp4'
    await show()
    button('Share a clip').click()
    await settle()
    await settle()
    flushSync()
    expect(calls('pick_clip_file')).toHaveLength(1)
    expect(calls('open_clip')).toEqual([['open_clip', { path: '/v/picked.mp4' }]])
    expect(editor()).not.toBeNull()
  })

  it('cancelling the picker opens nothing', async () => {
    picked = null
    await show()
    button('Share a clip').click()
    await settle()
    await settle()
    flushSync()
    expect(calls('open_clip')).toHaveLength(0)
    expect(editor()).toBeNull()
  })

  it('on linux share says windows-only and opens nothing', async () => {
    support = { can_send: false, reason: 'Sharing clips is Windows-only for now.' }
    await show()
    button('Share a clip').click()
    await settle()
    await settle()
    flushSync()
    expect(document.body.textContent).toContain('Sharing clips is Windows-only for now.')
    expect(calls('pick_clip_file')).toHaveLength(0)
    expect(calls('open_clip')).toHaveLength(0)
    // a drop is refused the same way
    await dragEvent({ type: 'drop', paths: ['/v/a.mp4'] })
    expect(calls('open_clip')).toHaveLength(0)
    expect(editor()).toBeNull()
  })

  it('an unreadable file says so', async () => {
    opens = async () => { throw "Pulse couldn't read that video." }
    await show()
    await dragEvent({ type: 'drop', paths: ['/v/notes.txt'] })
    expect(document.body.textContent).toContain("Pulse couldn't read that video.")
    expect(editor()).toBeNull()
    // whatever the failure, it says the same
    opens = async () => { throw new Error('probe exploded') }
    await dragEvent({ type: 'drop', paths: ['/v/broken.mp4'] })
    expect(document.body.textContent).toContain("Pulse couldn't read that video.")
    expect(document.body.textContent).not.toContain('probe exploded')
  })

  it("a job for another channel isn't shown here", async () => {
    app.state = setClipJob(setClipJob(app.state, job('J1', 'C1')), job('J2', 'C9'))
    await show() // the wall: newest top-left
    const pending = () => [...document.querySelectorAll<HTMLElement>('.pending-clip')]
    expect(pending()).toHaveLength(1)
    expect(pending()[0].textContent).toContain('sending J1')
    expect(document.body.textContent).not.toContain('sending J2')
    const wall = [...document.querySelectorAll<HTMLElement>('.grid > *')]
    expect(wall[0].classList.contains('pending-clip')).toBe(true)
    expect(wall[1].dataset.clipId).toBe('M2')

    // the list: newest at the bottom
    unmount(cmp!)
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    app.state = setClipJob(app.state, job('J1', 'C1'))
    await show()
    const list = [...document.querySelectorAll<HTMLElement>('.items > *')]
    expect(list.at(-1)!.querySelector('.pending-clip')).not.toBeNull()
    expect(list.at(-2)!.dataset.clipId).toBe('M2')

    // cancelled: gone
    app.state = setClipJob(app.state, job('J1', 'C1', { stage: 'done' }))
    flushSync()
    expect(pending()).toHaveLength(0)
  })

  it('a posted clip takes its card\'s place without a gap', async () => {
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    app.state = setClipJob(app.state, job('J1', 'C1', { stage: 'posting', progress: 1 }))
    await show()
    const pending = () => [...document.querySelectorAll<HTMLElement>('.pending-clip')]
    // the core says it posted before the clip itself arrives: the card stays
    app.state = setClipJob(app.state, job('J1', 'C1', { stage: 'done', message_id: 'M3' }))
    flushSync()
    expect(pending()).toHaveLength(1)
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: clip('M3'), nonce: null } }, Date.now())
    flushSync()
    expect(pending()).toHaveLength(0)
    expect(document.querySelector('[data-clip-id="M3"]')).not.toBeNull()
  })

  it('leaving the channel with the editor open lets its source go', async () => {
    await show()
    await dragEvent({ type: 'drop', paths: ['/v/a.mp4'] })
    expect(editor()).not.toBeNull()
    unmount(cmp!)
    cmp = undefined
    await settle()
    expect(calls('close_clip_source')).toEqual([['close_clip_source', { source: 'SRC1' }]])
  })

  it('an empty channel with a clip on its way does not say there are none', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [] : cmd === 'clip_facets' ? { games: [], authors: [] } : undefined))
    app.state = setClipJob(app.state, job('J1', 'C1'))
    await show()
    expect(document.querySelectorAll('.pending-clip')).toHaveLength(1)
    expect(document.body.textContent).not.toContain('No clips yet')
    app.state = setClipJob(app.state, job('J1', 'C1', { stage: 'done' }))
    flushSync()
    expect(document.body.textContent).toContain('No clips yet. Share one!')
  })
})
