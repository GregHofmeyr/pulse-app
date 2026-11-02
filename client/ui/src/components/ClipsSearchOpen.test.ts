// @vitest-environment jsdom
// A search result in a clips channel: there is no history list to jump in, so the clip's panel opens.
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

  it("leaves another channel's request alone", () => {
    app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }]))
    ui.openClip = { channelId: 'C9', clip: { ...clip, channel_id: 'C9' } }
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    expect(document.querySelector('.replies-panel')).toBeNull()
    expect(ui.openClip).not.toBeNull()
  })
})
