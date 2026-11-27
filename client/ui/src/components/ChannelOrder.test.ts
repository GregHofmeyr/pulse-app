// @vitest-environment jsdom
// Your own channel order (#304): drawn over the server's layout for you only; moves then change only yours (never a
// layout save); the server's later changes still show (new channels after yours); the Archived group stays apart;
// Reset to server order; the server tab's menu starts and resets it.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import ChannelList from './ChannelList.svelte'
import ServerMenu from './ServerMenu.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { resetSaves } from '../lib/layoutDraft.svelte'
import { testReady } from '../lib/testReady'
import type { Category } from '../lib/protocol/Category'
import type { Channel } from '../lib/protocol/Channel'
import type { ChannelOrder } from '../lib/protocol/ChannelOrder'
import { settle } from '../lib/testSettle'

const cat = (id: string, name: string, position: number): Category => ({ id, server_id: 'S1', name, position })
const cats: Category[] = [cat('C1', 'Text Channels', 0), cat('C2', 'Gaming', 1)]
const live: Channel[] = [
  { id: 'T0', server_id: 'S1', kind: 'text', name: 'welcome', position: 0 },
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0, category_id: 'C1' },
  { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1, category_id: 'C1' },
  { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0, category_id: 'C2' },
  { id: 'A1', server_id: 'S1', kind: 'text', name: 'old', position: 2, category_id: 'C1', archived_at: '2026-11-01T00:00:00.000Z' } as Channel,
]
const layouts = [{ server_id: 'S1', revision: 3, categories: cats, channels: [] }]
const mine: ChannelOrder = { server_id: 'S1', uncategorised: [], categories: [{ id: 'C2', channels: ['Q1', 'T0'] }, { id: 'C1', channels: ['T2', 'T1'] }] }

let cmp: ReturnType<typeof mount> | undefined
function show(orders: ChannelOrder[] = [mine]) {
  resetSaves()
  app.state = applyReady(emptyState(), testReady(live, { layouts, channel_orders: orders }))
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect: () => {}, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  invoke.mockReset()
})
const sections = () => [...document.querySelectorAll('.list > .cat')].map((c) => [...c.querySelectorAll('.row .grow')].map((e) => e.textContent?.trim()))
const headings = () => [...document.querySelectorAll('.list > .cat button.section')].map((h) => h.textContent?.trim())
const rowOf = (id: string) => document.querySelector<HTMLElement>(`.row[data-channel="${id}"]`)!
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a)

describe('your own channel order', () => {
  it('draws your order over the server’s, says so, and keeps archived channels apart', () => {
    show()
    expect(headings()).toEqual(['Gaming', 'Text Channels'])
    expect(sections()).toEqual([[], ['quotes', 'welcome'], ['memes', 'general']])
    expect(document.querySelector('[data-personal-order]')!.textContent).toContain('Your own order')
    expect(document.querySelector('[data-archived]')).not.toBeNull()
  })

  it('a move changes only yours, never the server’s layout', async () => {
    invoke.mockImplementation(async (c: string, a: { request: unknown }) => (c === 'set_channel_order' ? { server_id: 'S1', ...(a.request as object) } : undefined))
    show()
    rowOf('T2').focus()
    rowOf('T2').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', altKey: true, bubbles: true, cancelable: true }))
    await settle(6)
    expect(calls('save_layout')).toEqual([])
    expect(calls('set_channel_order')).toEqual([{ serverId: 'S1', request: { uncategorised: [], categories: [{ id: 'C2', channels: ['Q1', 'T0'] }, { id: 'C1', channels: ['T1', 'T2'] }] } }])
    expect(sections()[2]).toEqual(['general', 'memes'])
  })

  it('a refused move goes back', async () => {
    invoke.mockImplementation(async (c: string) => {
      if (c === 'set_channel_order') throw { kind: 'http', status: 429, message: 'Too fast' }
    })
    show()
    rowOf('T2').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', altKey: true, bubbles: true, cancelable: true }))
    await settle(6)
    expect(sections()[2]).toEqual(['memes', 'general'])
  })

  it('a channel the server adds later shows after yours; Reset goes back to the server’s order', async () => {
    invoke.mockResolvedValue(null)
    show()
    app.state = applyEvent(app.state, { t: 'ChannelCreated', d: { channel: { id: 'T3', server_id: 'S1', kind: 'text', name: 'new', position: 2, category_id: 'C1' } } }, 0)
    flushSync()
    expect(sections()[2]).toEqual(['memes', 'general', 'new'])
    ;[...document.querySelectorAll<HTMLButtonElement>('[data-personal-order] button')][0].click()
    await settle(6)
    expect(calls('set_channel_order')).toEqual([{ serverId: 'S1', request: null }])
    expect(headings()).toEqual(['Text Channels', 'Gaming'])
    expect(document.querySelector('[data-personal-order]')).toBeNull()
  })

  it('without one, moves save the server’s layout as before', async () => {
    invoke.mockImplementation((c: string) => (c === 'save_layout' ? new Promise(() => {}) : Promise.resolve(undefined)))
    show([])
    rowOf('T1').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', altKey: true, bubbles: true, cancelable: true }))
    await settle(6)
    expect(calls('save_layout')).toHaveLength(1)
    expect(calls('set_channel_order')).toEqual([])
  })
})

describe('the server tab’s menu', () => {
  const open = () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    cmp = mount(ServerMenu, { target: document.body, props: { serverId: 'S1', name: 'Main', x: 0, y: 0, opener, onClose: () => {}, onSettings: () => {}, onError: () => {} } })
    flushSync()
  }
  const item = () => document.querySelector<HTMLButtonElement>('[data-item="own-order"]')!

  it('Arrange channels just for me starts yours from the list as it is', async () => {
    invoke.mockImplementation(async (c: string, a: { request: unknown }) => (c === 'set_channel_order' ? { server_id: 'S1', ...(a.request as object) } : undefined))
    app.state = applyReady(emptyState(), testReady(live, { layouts }))
    open()
    expect(item().textContent).toBe('Arrange channels just for me')
    item().click()
    await settle(6)
    expect(calls('set_channel_order')).toEqual([{ serverId: 'S1', request: { uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T1', 'T2'] }, { id: 'C2', channels: ['Q1'] }] } }])
    expect(app.state.channelOrders.S1).toBeDefined()
  })

  it('with yours: Reset to server order', () => {
    invoke.mockResolvedValue(null)
    app.state = applyReady(emptyState(), testReady(live, { layouts, channel_orders: [mine] }))
    open()
    expect(item().textContent).toBe('Reset to server order')
  })
})
