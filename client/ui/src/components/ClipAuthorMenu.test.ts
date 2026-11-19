// @vitest-environment jsdom
// A clip's poster gets the person menu (#335): right-click their name or avatar, in the list and on the wall, or
// ContextMenu / Shift+F10 on the name.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ClipItem from './ClipItem.svelte'
import ClipCard from './ClipCard.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const clip: Message = {
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'that jump', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [],
  clip: { thumb_hash: 'a'.repeat(64), duration_ms: 30000, width: 1280, height: 720, size: 1000, game_id: null, reply_count: 0, last_reply_id: null, replies_seen: null },
}

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'clips', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
  }))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const menuOf = () => document.querySelector('[role=menu]')?.getAttribute('aria-label')
const zones = () => [...document.querySelectorAll<HTMLElement>('.person-zone')]
const props = { message: clip, serverId: 'S1', meId: 'U1', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'Someone') }

describe("a clip's poster (#335)", () => {
  it('in the list: right-click the avatar or the name opens their menu', () => {
    cmp = mount(ClipItem, { target: document.body, props })
    flushSync()
    expect(zones()).toHaveLength(2)
    for (const z of zones()) {
      const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 })
      z.firstElementChild!.dispatchEvent(e)
      flushSync()
      expect(e.defaultPrevented).toBe(true)
      expect(menuOf()).toBe('Actions for sam')
    }
  })

  it('on the wall: Shift+F10 on the name opens it', () => {
    cmp = mount(ClipCard, { target: document.body, props: { ...props, playing: false, onPlay: () => {} } })
    flushSync()
    const name = zones()[1].querySelector<HTMLElement>('button, .who')!
    name.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
    flushSync()
    expect(menuOf()).toBe('Actions for sam')
  })
})
