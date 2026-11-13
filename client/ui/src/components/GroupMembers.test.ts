// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (..._a: unknown[]): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import GroupMembers from './GroupMembers.svelte'
import NicknameDialog from './NicknameDialog.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { closeCard, closeRename, profileUi } from '../lib/profileCard.svelte'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const [alex, sam, kim, jo] = [user('U1', 'alex'), user('U2', 'sam'), user('U3', 'kim'), user('U4', 'jo')]
const p = (u: typeof alex, status: string) => ({ user: u, online: status !== 'offline', last_seen_at: null, status })

let cmp: ReturnType<typeof mount> | undefined
let dialog: ReturnType<typeof mount> | undefined
const onMessage = vi.fn()
beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  onMessage.mockReset()
  profileUi.rename = null
  closeCard()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  if (dialog) unmount(dialog)
  cmp = dialog = undefined
  document.body.innerHTML = ''
  closeCard()
})

function show(people: Record<string, ReturnType<typeof p>>) {
  app.state = { ...emptyState(), me: alex, dmMembers: { G1: Object.keys(people) }, people } as never
  cmp = mount(GroupMembers, { target: document.body, props: { channelId: 'G1', onMessage } })
  flushSync()
}
const labels = () => [...document.querySelectorAll('.members .label')].map((l) => l.textContent?.trim())
const order = () =>
  [...document.querySelectorAll('.members .label, .members .row')].map((e) => (e.classList.contains('label') ? e.textContent?.trim() : e.querySelector('.name')!.textContent))

describe('GroupMembers Online and Offline sections (#170)', () => {
  it('Online (DND included) above Offline, each by name, with counts; offline rows dimmed', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'dnd'), U3: p(kim, 'offline'), U4: p(jo, 'invisible') })
    expect(order()).toEqual(['Online — 2', 'alex', 'sam', 'Offline — 2', 'jo', 'kim'])
    for (const r of document.querySelectorAll<HTMLElement>('.members .row')) {
      expect(r.classList.contains('off')).toBe(['jo', 'kim'].includes(r.querySelector('.name')!.textContent!))
    }
  })

  it('every avatar has a dot; nobody else is ever drawn Invisible (#169, #171)', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'invisible') })
    expect(document.querySelectorAll('.members .row .pres')).toHaveLength(2)
    expect(document.querySelector('.members .pres.invisible')).toBeNull()
  })

  it('a section with nobody has no heading', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'online') })
    expect(labels()).toEqual(['Online — 2'])
  })
})

describe('GroupMembers row menu (#135): the server member list\'s person menu, with no server', () => {
  const menu = () => document.querySelector<HTMLElement>('[role=menu]')
  const items = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')]
  const ids = () => items().map((i) => i.dataset.item)
  const label = (id: string) => document.querySelector(`[role=menu] [data-item="${id}"]`)!.textContent
  const pickItem = (id: string) => {
    document.querySelector<HTMLElement>(`[role=menu] [data-item="${id}"]`)!.click()
    flushSync()
  }
  const rowOf = (name: string) => [...document.querySelectorAll<HTMLElement>('.members .row')].find((r) => r.querySelector('.name')!.textContent === name)!
  const rightClick = (el: Element) => {
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
    flushSync()
  }
  const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    flushSync()
  }
  const three = () => show({ U1: p(alex, 'online'), U2: p(sam, 'online'), U3: p(kim, 'offline') })

  it('right-click on a row: View profile, Message, Change nickname (your private one), then the other person actions', () => {
    three()
    rightClick(rowOf('sam'))
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for sam')
    expect(ids()).toEqual(['profile', 'message', 'private-nick', 'live-notify', 'appear-offline', 'remove'])
    expect(label('private-nick')).toBe('Change nickname')
    expect(document.activeElement).toBe(items()[0])
  })

  it('never a server nickname or server voice item: a group has no server', () => {
    three()
    rightClick(rowOf('kim'))
    expect(ids()).not.toContain('nick')
    expect(ids().some((i) => i!.startsWith('server-'))).toBe(false)
  })

  it('your own row: just your profile', () => {
    three()
    rightClick(rowOf('alex'))
    expect(ids()).toEqual(['profile'])
  })

  it('no hover X on the rows: Remove from group lives in the menu (#335)', () => {
    three()
    expect(document.querySelector('.members .row .x')).toBeNull()
    expect(document.querySelector('.members [aria-label^="Remove"], .members [data-tip="Remove from group"]')).toBeNull()
  })

  it('Remove from group (#335): last, in danger red, after a line; removes them from this group', async () => {
    three()
    rightClick(rowOf('kim'))
    expect(ids().at(-1)).toBe('remove')
    const item = document.querySelector<HTMLElement>('[role=menu] [data-item="remove"]')!
    expect(item.textContent?.trim()).toBe('Remove from group')
    expect(item.classList.contains('danger')).toBe(true)
    expect(item.getAttribute('role')).toBe('menuitem')
    expect(item.previousElementSibling?.getAttribute('role')).toBe('separator')
    pickItem('remove')
    await tick()
    expect(invoke.mock.calls.filter(([c]) => c === 'remove_member').map(([, a]) => a)).toEqual([{ channelId: 'G1', userId: 'U3' }])
    expect(menu()).toBeNull()
  })

  it('Remove from group: never on your own row (Leave group is the conversation menu\'s)', () => {
    three()
    rightClick(rowOf('alex'))
    expect(ids()).not.toContain('remove')
  })

  it('Remove from group: a failure shows under the list', async () => {
    three()
    invoke.mockImplementation(async (c: unknown) => { if (c === 'remove_member') throw 'not allowed'; return undefined })
    rightClick(rowOf('sam'))
    pickItem('remove')
    await tick(); await tick(); flushSync()
    expect(document.querySelector('.members [role=alert]')?.textContent).toBeTruthy()
  })

  it('Remove from group from the keyboard: Shift+F10, End, Enter', async () => {
    three()
    const name = rowOf('sam').querySelector<HTMLElement>('.name')!
    name.focus()
    key(name, 'F10', { shiftKey: true })
    key(document.activeElement!, 'End')
    expect((document.activeElement as HTMLElement).dataset.item).toBe('remove')
    key(document.activeElement!, 'Enter')
    await tick()
    expect(invoke.mock.calls.filter(([c]) => c === 'remove_member').map(([, a]) => a)).toEqual([{ channelId: 'G1', userId: 'U2' }])
  })

  it('View profile opens their card outside any server; Message messages them', () => {
    three()
    rightClick(rowOf('sam'))
    pickItem('profile')
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: null })
    expect(menu()).toBeNull()
    rightClick(rowOf('sam'))
    pickItem('message')
    expect(onMessage).toHaveBeenCalledWith('U2')
  })

  it('a private nickname: set from the menu, shown on the row, then cleared in the same dialog', async () => {
    three()
    rightClick(rowOf('sam'))
    pickItem('private-nick')
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
    dialog = mount(NicknameDialog, { target: document.body, props: { target: profileUi.rename!, onClose: closeRename } })
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Nickname"]')!
    input.value = 'Sammy'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    document.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }))
    await tick(); await tick(); flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'set_private_nickname').map(([, a]) => a)).toEqual([{ userId: 'U2', nickname: 'Sammy' }])
    expect(profileUi.rename).toBeNull()
    unmount(dialog)
    dialog = undefined

    // the server confirms: the row goes by it, and the menu still says Change nickname
    app.state = { ...app.state, privateNicks: { U2: 'Sammy' } }
    flushSync()
    rightClick(rowOf('Sammy'))
    expect(label('private-nick')).toBe('Change nickname')
    pickItem('private-nick')
    dialog = mount(NicknameDialog, { target: document.body, props: { target: profileUi.rename!, onClose: closeRename } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === 'Clear')!.click()
    await tick(); await tick(); flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'set_private_nickname').at(-1)![1]).toEqual({ userId: 'U2', nickname: null })
  })

  it('the keyboard: ContextMenu or Shift+F10 on the name opens it; Esc closes it and focus goes back', () => {
    three()
    const name = rowOf('sam').querySelector<HTMLElement>('.name')!
    name.focus()
    key(name, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    expect(document.activeElement).toBe(items()[0])
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(name)
    key(name, 'ContextMenu')
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'ArrowDown')
    key(document.activeElement!, 'Enter')
    expect(onMessage).toHaveBeenCalledWith('U2')
    expect(menu()).toBeNull()
  })
})
