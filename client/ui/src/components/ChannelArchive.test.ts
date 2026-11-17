// @vitest-environment jsdom
// Archiving a channel (#303): the owner's Archive channel / Unarchive channel on a text channel's menu, the collapsed
// Archived group at the bottom of the list, and the read-only note where the composer was.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ChannelList from './ChannelList.svelte'
import MessageItem from './MessageItem.svelte'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'
import type { Message } from '../lib/protocol/Message'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { sidebarLayout, orderOf } from '../lib/layout'
import { testReady } from '../lib/testReady'
import type { Channel } from '../lib/protocol/Channel'

const channels = (archived: boolean): Channel[] => [
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
  { id: 'T2', server_id: 'S1', kind: 'text', name: 'old-news', position: 1, ...(archived ? { archived_at: '2026-11-01T00:00:00.000Z' } : {}) },
  { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
]

let cmp: ReturnType<typeof mount> | undefined
function render({ archived = false, owner = 'U1', active = null as string | null } = {}) {
  if (cmp) unmount(cmp)
  app.state = applyReady(emptyState(), testReady(channels(archived), {
    servers: [{ id: 'S1', name: 'Main', icon_hash: null, owner_id: owner }],
    layouts: [{ server_id: 'S1', revision: 1, categories: [], channels: [] }],
  } as never))
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: active, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}
const rows = () => [...document.querySelectorAll<HTMLElement>('.row')].map((r) => r.textContent?.trim())
const row = (name: string) => [...document.querySelectorAll<HTMLElement>('.row')].find((r) => r.textContent?.includes(name))!
const items = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')]
const item = (id: string) => items().find((b) => b.dataset.item === id)
function rightClick(name: string) {
  row(name).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('the layout', () => {
  it('keeps archived channels out of the sections and the order, in their own list', () => {
    app.state = applyReady(emptyState(), testReady(channels(true), { layouts: [{ server_id: 'S1', revision: 1, categories: [], channels: [] }] } as never))
    const l = sidebarLayout(app.state, 'S1')
    expect(l.archived.map((c) => c.id)).toEqual(['T2'])
    expect(l.top.map((c) => c.id)).not.toContain('T2')
    expect(orderOf(l).top).not.toContain('T2')
  })
})

describe('the Archived group', () => {
  it('sits at the bottom, collapsed, and opens', () => {
    render({ archived: true })
    const group = document.querySelector<HTMLElement>('[data-archived]')!
    expect(group).not.toBeNull()
    expect(group.textContent).toContain('Archived')
    expect(rows()).not.toContain('old-news')
    const toggle = group.querySelector<HTMLButtonElement>('button.section')!
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    toggle.click()
    flushSync()
    expect(rows().at(-1)).toBe('old-news')
  })

  it('shows the open channel even while collapsed, and isn\'t there with nothing archived', () => {
    render({ archived: true, active: 'T2' })
    expect(rows()).toContain('old-news')
    render()
    expect(document.querySelector('[data-archived]')).toBeNull()
  })
})

describe('Archive channel on the menu', () => {
  it('the owner archives a text channel', async () => {
    render()
    rightClick('old-news')
    item('archive')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('set_channel_archived', { channelId: 'T2', archived: true }))
  })

  it('and unarchives an archived one', async () => {
    render({ archived: true, active: 'T2' })
    rightClick('old-news')
    expect(item('archive')).toBeUndefined()
    item('unarchive')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('set_channel_archived', { channelId: 'T2', archived: false }))
  })

  it('nobody else gets it, nor a voice channel', () => {
    render({ owner: 'U2' })
    rightClick('old-news')
    expect(item('archive')).toBeUndefined()
    render()
    rightClick('Lounge')
    expect(item('archive')).toBeUndefined()
  })
})

describe('a message in an archived channel', () => {
  it('offers no reacting, replying or editing', () => {
    const m: Message = {
      id: 'M1', channel_id: 'T2', author_id: 'U1', kind: 'normal', content: 'hello', reply_to_id: null,
      created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
      quote: null, qoty_card: null, clip: null, embeds: [],
    }
    const show = (archived: boolean) => {
      if (cmp) unmount(cmp)
      app.state = applyReady(emptyState(), testReady(channels(archived)))
      cmp = mount(MessageItem, { target: document.body, props: {
        message: m, serverId: 'S1', name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null,
        grouped: false, mine: true, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
      } })
      flushSync()
      return [...document.querySelectorAll('.actions button')].map((b) => b.getAttribute('aria-label'))
    }
    expect(show(false)).toEqual(expect.arrayContaining(['Add reaction', 'Reply', 'Edit']))
    const archived = show(true)
    expect(archived).not.toContain('Add reaction')
    expect(archived).not.toContain('Reply')
    expect(archived).not.toContain('Edit')
    // deleting your own message stays (#418): the server allows it too
    expect(archived).toContain('Delete')
  })

  const showWith = (over: Partial<Message>) => {
    const m: Message = {
      id: 'M1', channel_id: 'T2', author_id: 'U1', kind: 'normal', content: 'see https://example.com/a', reply_to_id: null,
      created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
      quote: null, qoty_card: null, clip: null, embeds: [], ...over,
    }
    if (cmp) unmount(cmp)
    app.state = applyReady(emptyState(), testReady(channels(true)))
    cmp = mount(MessageItem, { target: document.body, props: {
      message: m, serverId: 'S1', name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null,
      grouped: false, mine: true, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
    } })
    flushSync()
  }

  it('shows reaction pills that can\'t be clicked, and nothing to add one with (#418)', () => {
    invoke.mockResolvedValue([])
    showWith({ reactions: [{ emoji: '👍', count: 2, me: true }, { emoji: '🎉', count: 1, me: false }] })
    const chips = [...document.querySelectorAll<HTMLButtonElement>('.reactions .chip')]
    expect(chips).toHaveLength(2)
    for (const c of chips) {
      expect(c.getAttribute('aria-disabled')).toBe('true')
      expect(c.classList.contains('locked')).toBe(true)
      c.click()
    }
    expect(invoke).not.toHaveBeenCalledWith('add_reaction', expect.anything())
    expect(invoke).not.toHaveBeenCalledWith('remove_reaction', expect.anything())
    expect(document.querySelector('.reactions .chip.add')).toBeNull()
  })

  it('offers nothing to remove its previews with (#418)', () => {
    showWith({ embeds: [{ url: 'https://example.com/a', provider: 'generic', kind: 'card', title: 'A' }] as never })
    expect(document.querySelector('button[aria-label="Remove previews"]')).toBeNull()
    expect(document.querySelector('button[aria-label="Remove preview"]')).toBeNull()
  })
})
