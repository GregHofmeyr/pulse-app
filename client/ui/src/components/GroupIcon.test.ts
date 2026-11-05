// @vitest-environment jsdom
// Group DM icons (#56): any member sets or clears it from the conversation menu (right-click or ⋯), through the
// avatar upload and crop; the DM list and the header show it, with today's stacked faces as the fallback.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown, _opts?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never, opts?: never) => invoke(cmd, args, opts) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ConversationMenu from './ConversationMenu.svelte'
import ConversationList from './ConversationList.svelte'
import TextChannel from './TextChannel.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import { applyGroupIcon, groupIcon, pickGroupIcon } from '../lib/groupIcon.svelte'
import { mediaUrl } from '../lib/emoji'
import type { Channel } from '../lib/protocol/Channel'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const HASH = 'ab'.repeat(32)
const STILL = 'cd'.repeat(32)
const group = (over: Partial<Channel> = {}): Channel => ({ id: 'G1', server_id: null, kind: 'group', name: 'Squad', position: 0, ...over })
const dm: Channel = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 }

let cmp: ReturnType<typeof mount> | undefined
function load(g: Channel) {
  app.state = applyReady(emptyState(), testReady([g, dm], {
    people: [testMe, user('U2', 'sam'), user('U3', 'kim')].map((u) => ({ user: u, online: true, last_seen_at: null, status: 'online' as const })),
    dm_members: [{ channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }, { channel_id: 'D1', user_ids: ['U1', 'U2'] }],
    read_states: [{ channel_id: 'G1', last_read_message_id: null, unread: 0, mentions: 0 }, { channel_id: 'D1', last_read_message_id: null, unread: 0, mentions: 0 }],
  }))
}
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a)
const items = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] .menu-item')].map((b) => b.textContent?.trim())
const item = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] .menu-item')].find((b) => b.textContent?.trim() === t)

// the core as TextChannel needs it: an empty history
const core = async (cmd: string): Promise<unknown> => (cmd === 'list_messages' ? [] : undefined)
beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockImplementation(core)
  Element.prototype.scrollTo = () => {} // jsdom has none
  groupIcon.pending = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('the conversation menu', () => {
  const menu = (channel: Channel, show: 'manage' | 'all') => {
    cmp = mount(ConversationMenu, { target: document.body, props: { channel, muted: false, show, at: { x: 10, y: 10 }, onClose: () => {}, onLeft: () => {} } })
    flushSync()
  }

  it('a group without an icon: Change icon, no Remove icon (right-click and ⋯ alike)', () => {
    load(group())
    for (const show of ['all', 'manage'] as const) {
      menu(group(), show)
      expect(items()).toContain('Change icon')
      expect(items()).not.toContain('Remove icon')
      unmount(cmp!)
      cmp = undefined
    }
  })

  it('a group with an icon: Remove icon clears it', async () => {
    load(group({ icon_hash: HASH }))
    menu(group({ icon_hash: HASH }), 'all')
    item('Remove icon')!.click()
    await Promise.resolve()
    expect(calls('set_group_icon')).toEqual([{ channelId: 'G1', hash: null }])
  })

  it('a DM has neither', () => {
    load(group())
    menu(dm, 'all')
    expect(items().filter((t) => /icon/i.test(t ?? ''))).toEqual([])
  })

  it('Change icon opens a picture picker for that group', () => {
    load(group())
    const click = vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (this: HTMLInputElement) {
      Object.defineProperty(this, 'files', { value: [new File(['x'], 'cat.png', { type: 'image/png' })] })
      this.dispatchEvent(new Event('change'))
    })
    menu(group(), 'all')
    item('Change icon')!.click()
    expect(click).toHaveBeenCalledOnce()
    expect(groupIcon.pending?.channelId).toBe('G1')
    expect(groupIcon.pending?.file.name).toBe('cat.png')
  })
})

describe('setting it', () => {
  it('uploads through the avatar pipeline with the crop, then sets the group icon', async () => {
    load(group())
    invoke.mockImplementation(async (cmd: string) => (cmd === 'upload_avatar_media' ? { hash: HASH, still_hash: null, frames: 1 } : core(cmd)))
    pickGroupIcon('G1', new File([new Uint8Array([1, 2, 3])], 'cat.png', { type: 'image/png' }))
    await applyGroupIcon({ x: 1, y: 2, w: 30, h: 30 })
    const up = invoke.mock.calls.find(([c]) => c === 'upload_avatar_media')!
    expect(up[2]).toEqual({ headers: { 'x-pulse-crop': '1,2,30,30' } })
    expect(calls('set_group_icon')).toEqual([{ channelId: 'G1', hash: HASH }])
    expect(groupIcon.pending).toBeNull()
  })

  it('a refusal says why', async () => {
    load(group())
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'upload_avatar_media') throw { kind: 'http', status: 400, message: 'That picture is too big.' }
      return undefined
    })
    pickGroupIcon('G1', new File(['x'], 'big.png', { type: 'image/png' }))
    await applyGroupIcon({ x: 0, y: 0, w: 10, h: 10 })
    expect(calls('set_group_icon')).toEqual([])
    expect(ui.notice).toBeTruthy()
  })
})

describe('where it shows', () => {
  const img = (root: ParentNode) => root.querySelector<HTMLImageElement>('.gicon img')

  it('the DM list: the icon (its still) instead of the stacked faces; without one, the faces', () => {
    load(group({ icon_hash: HASH, icon_still: STILL }))
    cmp = mount(ConversationList, { target: document.body, props: { activeId: null, onOpen: () => {}, onNew: () => {}, onLeft: () => {} } })
    flushSync()
    const row = [...document.querySelectorAll('.row')].find((r) => r.textContent?.includes('Squad'))!
    expect(img(row)!.getAttribute('src')).toBe(mediaUrl(STILL))
    expect(row.querySelector('.stack')).toBeNull()
    // cleared by another member: back to the faces
    app.state = applyEvent(app.state, { t: 'ChannelUpdated', d: { channel: group() } }, 0)
    flushSync()
    const again = [...document.querySelectorAll('.row')].find((r) => r.textContent?.includes('Squad'))!
    expect(img(again)).toBeNull()
    expect(again.querySelector('.stack')).not.toBeNull()
  })

  it('the header: the icon instead of the pile', () => {
    load(group({ icon_hash: HASH }))
    cmp = mount(TextChannel, { target: document.body, props: { channelId: 'G1', serverId: null } })
    flushSync()
    const head = document.querySelector('.head')!
    expect(img(head)!.getAttribute('src')).toBe(mediaUrl(HASH))
    expect(head.querySelector('.pile')).toBeNull()
  })
})
