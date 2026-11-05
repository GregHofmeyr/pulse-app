// @vitest-environment jsdom
// Right-click a DM in the conversation list (#137): Change nickname sets your private nickname for them.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ConversationList from './ConversationList.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { profileUi } from '../lib/profileCard.svelte'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  resetState()
  profileUi.rename = null
  app.state = {
    ...emptyState(), me: alex,
    people: Object.fromEntries([alex, sam, kim].map((u) => [u.id, { user: u, online: true, last_seen_at: null }])),
    // in a server with sam too: still no server nickname item here
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] },
    channels: {
      D1: { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
      G1: { id: 'G1', server_id: null, kind: 'group', name: 'squad', position: 0 },
    },
    dmMembers: { D1: ['U1', 'U2'], G1: ['U1', 'U2', 'U3'] },
  } as never
  cmp = mount(ConversationList, { target: document.body, props: { activeId: null, onOpen: () => {}, onNew: () => {}, onLeft: () => {} } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const rowFor = (text: string) => [...document.querySelectorAll<HTMLElement>('.list .row')].find((r) => r.textContent?.includes(text))!
const rightClick = (el: Element) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
  flushSync()
}
const item = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role=menuitem]')].find((b) => b.textContent?.trim() === text)
const texts = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role=menuitem]')].map((b) => b.textContent?.trim())

describe('the DM list menu (#137)', () => {
  it('a DM: Change nickname opens the dialog for your private nickname for them, and the menu closes', () => {
    rightClick(rowFor('sam'))
    expect(item('Change nickname')).toBeDefined()
    expect(texts().filter((t) => /nickname/i.test(t ?? ''))).toEqual(['Change nickname'])
    item('Change nickname')!.click()
    flushSync()
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
    expect(document.querySelector('[role=menu]')).toBeNull()
  })

  it('a group: no nickname item (it names no one person)', () => {
    rightClick(rowFor('squad'))
    expect(texts().filter((t) => /nickname/i.test(t ?? ''))).toEqual([])
  })

  it('a DM: Notify me when they go live, and back (#60); never in a group', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    rightClick(rowFor('sam'))
    item('Notify me when they go live')!.click()
    flushSync()
    expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === 'set_live_notify').map(([, a]) => a)).toEqual([{ userId: 'U2', on: true }])
    expect(document.querySelector('[role=menu]')).toBeNull()
    app.state = { ...app.state, liveNotify: ['U2'] }
    flushSync()
    rightClick(rowFor('sam'))
    expect(item('Stop notifying me when they go live')).toBeDefined()
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    rightClick(rowFor('squad'))
    expect(texts().filter((t) => /go live/i.test(t ?? ''))).toEqual([])
  })

  it('a DM: Appear offline to them, and back (#45); never in a group', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    rightClick(rowFor('sam'))
    item('Appear offline to them')!.click()
    flushSync()
    expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === 'set_appear_offline').map(([, a]) => a)).toEqual([{ userId: 'U2', hidden: true }])
    expect(document.querySelector('[role=menu]')).toBeNull()
    app.state = { ...app.state, appearOfflineTo: ['U2'] }
    flushSync()
    rightClick(rowFor('sam'))
    expect(item('Stop appearing offline to them')).toBeDefined()
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    rightClick(rowFor('squad'))
    expect(texts().filter((t) => /offline/i.test(t ?? ''))).toEqual([])
  })
})

describe('Mute in the DM list menu (#190): a durations flyout, as the channel menu has', () => {
  const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label^="Mute "]')
  const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }

  it('one Mute conversation item; the durations only open beside it', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    rightClick(rowFor('sam'))
    expect(texts()).not.toContain('For 15 minutes')
    const mute = item('Mute conversation')!
    expect(mute.getAttribute('aria-haspopup')).toBe('menu')
    expect(flyout()).toBeNull()
    hover(mute)
    expect([...flyout()!.querySelectorAll('[role=menuitem]')].map((b) => b.textContent?.trim()))
      .toEqual(['For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on'])
    ;[...flyout()!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.trim() === 'Until I turn it back on')!.click()
    flushSync()
    expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === 'set_mute').map(([, a]) => a)).toEqual([{ targetKind: 'channel', targetId: 'D1', until: null }])
    await vi.waitFor(() => expect(document.querySelector('[role=menu]')).toBeNull())
  })

  it('a group has the same flyout', () => {
    rightClick(rowFor('squad'))
    hover(item('Mute conversation')!)
    expect(flyout()).not.toBeNull()
  })

  it('muted: Unmute conversation instead', () => {
    app.state = { ...app.state, mutes: [{ target_kind: 'channel', target_id: 'D1', until: null }] } as never
    flushSync()
    rightClick(rowFor('sam'))
    expect(item('Unmute conversation')).toBeDefined()
    expect(item('Mute conversation')).toBeUndefined()
  })
})
