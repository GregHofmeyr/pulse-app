// @vitest-environment jsdom
// A search result in a clips channel: a clip is scrolled to in the feed and flashed (#120); the panel opens for a reply's
// clip, a pinned clip, or a clip the feed can't reach.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => `asset://${s}` }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ClipsChannel from './ClipsChannel.svelte'
import { app, ui } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const clip: Message = {
  id: 'V1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: 'the clutch', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [],
  clip: { thumb_hash: 'v'.padEnd(64, '0'), duration_ms: 65000, width: 1280, height: 720, size: 1000, game_id: null, reply_count: 0, last_reply_id: null, replies_seen: null },
}

// jsdom has no scrollTo (the replies panel scrolls to its newest): stub it, then put it back.
const realScrollTo = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollTo')
let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  Object.defineProperty(Element.prototype, 'scrollTo', { configurable: true, writable: true, value: () => {} })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  ui.openClip = null
  ui.jumpClip = null
  if (realScrollTo) Object.defineProperty(Element.prototype, 'scrollTo', realScrollTo)
  else delete (Element.prototype as { scrollTo?: unknown }).scrollTo
})

describe('a clip opened from search', () => {
  it("opens that clip's panel and clears the request", async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_replies' ? [] : cmd === 'clip_facets' ? { games: [], authors: [] } : undefined))
    app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
      members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    }))
    ui.openClip = { channelId: 'C1', clip }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.waitFor(() => expect(document.querySelector('.replies-panel')).not.toBeNull())
    expect(ui.openClip).toBeNull()
  })

  // #257: the same rule for a pinned clip: the header's pins list it, and Open shows its panel
  it("a pinned clip opens its panel from the header's pins", async () => {
    const pin = { pinned_by: 'U1', pinned_at: '2026-10-01T10:00:00.000Z', expires_at: null }
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'list_pins' ? [{ message: { ...clip, pin }, pin }]
      : cmd === 'list_replies' ? []
      : cmd === 'clip_facets' ? { games: [], authors: [] }
      : undefined)
    app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
      members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    }))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    const pinBtn = document.querySelector<HTMLButtonElement>('.head button[aria-label="Pinned clips"]')!
    expect(pinBtn.classList.contains('hbtn')).toBe(true)
    expect(pinBtn.querySelector('svg')?.getAttribute('width')).toBe('17')
    pinBtn.click()
    flushSync()
    expect(pinBtn.getAttribute('aria-expanded')).toBe('true')
    await vi.waitFor(() => expect(document.querySelector('.pins-pop .pin .said')?.textContent).toBe('the clutch'))
    ;[...document.querySelectorAll<HTMLButtonElement>('.pins-pop .pin button')].find((b) => b.textContent === 'Open')!.click()
    flushSync()
    expect(document.querySelector('.pins-pop')).toBeNull()
    await vi.waitFor(() => expect(document.querySelector('.replies-panel')).not.toBeNull())
    expect(invoke).toHaveBeenCalledWith('list_replies', expect.objectContaining({ messageId: 'V1' }))
  })

  it("leaves another channel's request alone", () => {
    app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }]))
    ui.openClip = { channelId: 'C9', clip: { ...clip, channel_id: 'C9' } }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    expect(document.querySelector('.replies-panel')).toBeNull()
    expect(ui.openClip).not.toBeNull()
  })
})

describe('a clip jumped to from search (#120)', () => {
  const older = (n: number): Message => ({ ...clip, id: `V0${n}`, content: `older ${n}` })
  const setup = (pages: Message[][]) => {
    const queue = [...pages]
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'list_clips' ? (queue.shift() ?? [])
      : cmd === 'list_replies' ? []
      : cmd === 'clip_facets' ? { games: [], authors: [] }
      : undefined)
    app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
      members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    }))
  }
  let scrolled: string[] = []
  const realScrollIntoView = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollIntoView')
  beforeEach(() => {
    scrolled = []
    Object.defineProperty(Element.prototype, 'scrollIntoView', {
      configurable: true, writable: true,
      value(this: HTMLElement) { if (this.dataset.clipId) scrolled.push(this.dataset.clipId) },
    })
  })
  afterEach(() => {
    if (realScrollIntoView) Object.defineProperty(Element.prototype, 'scrollIntoView', realScrollIntoView)
    else delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView
  })

  it('scrolls the feed to the clip and flashes it, without opening its panel', async () => {
    setup([[{ ...clip, id: 'V9' }, clip]])
    ui.jumpClip = { channelId: 'C1', clip }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.waitFor(() => expect(scrolled).toEqual(['V1']))
    expect(document.querySelector('[data-clip-id="V1"]')?.classList.contains('jump-flash')).toBe(true)
    expect(document.querySelector('.replies-panel')).toBeNull()
    expect(ui.jumpClip).toBeNull()
  })

  it('pages back through the feed until the clip is in it', async () => {
    // a full first page (30) without it: the feed has more, so it pages back once
    setup([Array.from({ length: 30 }, (_, i) => older(31 - i)), [older(1), clip]])
    ui.jumpClip = { channelId: 'C1', clip }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.waitFor(() => expect(scrolled).toEqual(['V1']))
    expect(invoke.mock.calls.filter((c) => c[0] === 'list_clips')).toHaveLength(2)
  })

  it("opens the clip's panel when the feed doesn't have it (gone, or past the start)", async () => {
    setup([[older(2)], []])
    ui.jumpClip = { channelId: 'C1', clip }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.waitFor(() => expect(document.querySelector('.replies-panel')).not.toBeNull())
    expect(scrolled).toEqual([])
  })
})
