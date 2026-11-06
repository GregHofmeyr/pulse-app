// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ServerSettings from './ServerSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ServerSettings dialog', () => {
  it('focuses into the dialog, closes on Esc, and gives focus back', () => {
    app.state = emptyState()
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    const dialog = document.querySelector('[role=dialog]')!
    expect(dialog.contains(document.activeElement)).toBe(true)
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
    unmount(cmp)
    cmp = undefined
    expect(document.activeElement).toBe(opener)
  })

  // Svelte delegates onkeydown to the mount root, so these run the real inner handlers first.
  const esc = (el: Element) => el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  const settle = () => new Promise((r) => setTimeout(r, 0))
  const emoji = { id: 'X1', server_id: 'S1', name: 'pog', hash: 'h1', still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z' }

  it('Esc in the add form closes the form, not the dialog', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ hash: 'nh', content_type: 'image/webp', width: 128, height: 128, frames: 1, animated: false, still_hash: null })
    app.state = emptyState()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    const file = document.querySelector<HTMLInputElement>('input[type=file]')!
    Object.defineProperty(file, 'files', { value: [new File([new Uint8Array([1])], 'a.png')], configurable: true })
    file.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    flushSync()
    esc(document.querySelector('input[aria-label="Emoji name"]')!)
    flushSync()
    expect(document.querySelector('input[aria-label="Emoji name"]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Esc in the rename input cancels the rename, not the dialog', async () => {
    app.state = { ...emptyState(), customEmoji: { X1: emoji } }
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Rename :pog:"]')!.click()
    flushSync()
    esc(document.querySelector('input[aria-label="Rename emoji"]')!)
    flushSync()
    expect(document.querySelector('input[aria-label="Rename emoji"]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
    esc(document.querySelector('[role=dialog]')!) // nothing inside handled it: now it closes
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('Esc with focus outside the dialog still closes it, and Tab pulls focus back in', () => {
    app.state = emptyState()
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    ;(document.activeElement as HTMLElement).blur()
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true })
    document.body.dispatchEvent(tab)
    expect(tab.defaultPrevented).toBe(true)
    expect(document.querySelector('[role=dialog]')!.contains(document.activeElement)).toBe(true)
    esc(document.body)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('lists links and undo calls unlink_speaker', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const kim = { id: 'U3', username: 'kim', display_name: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = {
      ...emptyState(), me,
      members: { S1: [{ user: me, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] },
      speakerLinks: { S1: [{ server_id: 'S1', name_key: 'a friend', name: 'a friend', user_id: 'U2', linked_by: 'U1', linked_at: '2026-10-01T00:00:00Z' }] },
    }
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Quote speakers')!.click()
    flushSync()
    const row = document.querySelector('.link')!
    expect(row.textContent).toContain('a friend')
    expect(row.textContent).toContain('sam')
    expect(row.textContent).toContain('linked by alex')

    // Change: pick another member by typing
    ;[...row.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Change')!.click()
    flushSync()
    // not offered the member it's already linked to
    expect([...document.querySelectorAll('.matches button')].map((b) => b.textContent)).toEqual(['alex', 'kim'])
    const find = document.querySelector<HTMLInputElement>('input[aria-label="Find a member"]')!
    find.value = 'ki'
    find.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.link button')].find((b) => b.textContent === 'kim')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('link_speaker', { serverId: 'S1', body: { name: 'a friend', user_id: 'U3' } })

    ;[...document.querySelectorAll<HTMLButtonElement>('.link button')].find((b) => b.textContent === 'Undo')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unlink_speaker', { serverId: 'S1', nameKey: 'a friend' })
  })

  it('Escape in the member picker closes the picker, not the dialog', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = {
      ...emptyState(), me,
      members: { S1: [{ user: me, nickname: null }, { user: sam, nickname: null }] },
      speakerLinks: { S1: [{ server_id: 'S1', name_key: 'a friend', name: 'a friend', user_id: 'U2', linked_by: 'U1', linked_at: '2026-10-01T00:00:00Z' }] },
    }
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Quote speakers')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.link button')].find((b) => b.textContent === 'Change')!.click()
    flushSync()
    esc(document.querySelector('input[aria-label="Find a member"]')!)
    flushSync()
    expect(document.querySelector('input[aria-label="Find a member"]')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('says when no names are linked', () => {
    app.state = emptyState()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Quote speakers')!.click()
    flushSync()
    expect(document.querySelector('.content')!.textContent).toContain('No names are linked yet')
  })
})

describe('ServerSettings sections', () => {
  it('ServerSettings has a Storage section', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue({ bytes: 0, count: 0, est_monthly_usd: 0 })
    app.state = emptyState()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Storage')!.click()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(invoke).toHaveBeenCalledWith('clip_storage', { serverId: 'S1' })
    expect(document.querySelector('.content h2')!.textContent).toBe('Storage')
  })

  it('ServerSettings has a Recently deleted section', async () => {
    invoke.mockReset()
    invoke.mockResolvedValue([])
    app.state = emptyState()
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Recently deleted')!.click()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(invoke).toHaveBeenCalledWith('list_deleted_channels', { serverId: 'S1' })
    expect(document.querySelector('.content h2')!.textContent).toBe('Recently deleted')
  })
})

// #203: the server's notification level, as the server tab's right-click menu offers it (#195).
describe('ServerSettings: Notifications', () => {
  type Level = { target_kind: 'server' | 'channel'; target_id: string; level: 'all' | 'mentions' | 'nothing' }
  const settle = () => new Promise((r) => setTimeout(r, 0))
  const open = (levels: Level[] = []) => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    app.state = { ...emptyState(), notifyLevels: levels }
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Home', icon_hash: null }, onClose: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Notifications')!.click()
    flushSync()
  }
  const radios = () => [...document.querySelectorAll<HTMLElement>('[role=radiogroup][aria-label="Notifications for Home"] [role=radio]')]
  const ticked = () => radios().filter((r) => r.getAttribute('aria-checked') === 'true').map((r) => r.getAttribute('aria-label'))

  it('lists the menu’s levels with yours ticked, All messages by default', () => {
    open()
    expect(document.querySelector('.content h2')!.textContent).toBe('Notifications')
    expect(radios().map((r) => r.getAttribute('aria-label'))).toEqual(['All messages', 'Only @mentions', 'Nothing'])
    expect(ticked()).toEqual(['All messages'])
    // the tick: the menus' check icon, on the chosen one only
    expect(radios().map((r) => !!r.closest('.level')!.querySelector('svg'))).toEqual([true, false, false])
    unmount(cmp!)
    open([{ target_kind: 'server', target_id: 'S1', level: 'mentions' }, { target_kind: 'server', target_id: 'S2', level: 'nothing' }])
    expect(ticked()).toEqual(['Only @mentions'])
  })

  it('picking one sets it; All messages puts the default back; the tick follows the change', async () => {
    open()
    radios()[2].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'server', targetId: 'S1', level: 'nothing' })
    app.state = { ...app.state, notifyLevels: [{ target_kind: 'server', target_id: 'S1', level: 'nothing' }] }
    flushSync()
    expect(ticked()).toEqual(['Nothing'])
    radios()[0].click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_notify_level', { targetKind: 'server', targetId: 'S1', level: null })
  })

  it('arrow keys move the choice, as the other settings radio groups do', async () => {
    open()
    radios()[0].focus()
    radios()[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'server', targetId: 'S1', level: 'mentions' })
    expect(document.activeElement).toBe(radios()[1])
  })

  it('says why when it fails', async () => {
    open()
    invoke.mockRejectedValue('rate limited')
    radios()[1].click()
    await settle()
    flushSync()
    expect(document.querySelector('.content [role=alert]')?.textContent).toContain('rate limited')
  })
})
