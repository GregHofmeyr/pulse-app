// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import NicknameDialog from './NicknameDialog.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { RenameTarget } from '../lib/profileCard.svelte'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'Sam 🎸', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)

let cmp: ReturnType<typeof mount> | undefined
let onClose: ReturnType<typeof vi.fn<() => void>>
function show(target: RenameTarget, nickname: string | null = null, privateNick?: string) {
  app.state = {
    ...emptyState(), me: alex,
    servers: [{ id: 'S1', name: 'Den', icon_hash: null }],
    people: { U2: { user: sam, online: true, last_seen_at: null } },
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname }] },
    privateNicks: privateNick ? { U2: privateNick } : {},
  } as never
  onClose = vi.fn<() => void>()
  cmp = mount(NicknameDialog, { target: document.body, props: { target, onClose } })
  flushSync()
}
const input = () => document.querySelector<HTMLInputElement>('input[aria-label="Nickname"]')!
const button = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === t)
const type = (v: string) => { input().value = v; input().dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
const submit = async () => { document.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true })); await tick(); await tick(); flushSync() }

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async () => undefined) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('NicknameDialog (#48)', () => {
  it('a server nickname: says who sees it, saves trimmed, and closes', async () => {
    show({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(document.body.textContent).toContain('Everyone in Den sees it')
    expect(input().placeholder).toBe('Sam 🎸')
    expect(button('Reset')).toBeUndefined() // nothing to reset
    type('  Big Dog  ')
    await submit()
    expect(calls('set_nickname')).toEqual([['set_nickname', { serverId: 'S1', userId: 'U2', nickname: 'Big Dog' }]])
    expect(onClose).toHaveBeenCalled()
  })

  it('a Zalgo nickname gets a soft "Zalgo? Really?" and still saves (#209)', async () => {
    show({ kind: 'private', userId: 'U2' })
    type('sam\u{300}\u{301}\u{302}')
    const hint = document.querySelector('.zalgo')!
    expect(hint.textContent).toBe('Zalgo? Really?')
    expect(document.querySelector('[role=alert]')).toBeNull()
    expect(button('Save')!.disabled).toBe(false)
    await submit()
    expect(calls('set_private_nickname')).toEqual([['set_private_nickname', { userId: 'U2', nickname: 'sam\u{300}\u{301}\u{302}' }]])
    type('sam')
    expect(document.querySelector('.zalgo')).toBeNull()
  })

  it('Reset (or a blank Save) removes it', async () => {
    show({ kind: 'server', userId: 'U2', serverId: 'S1' }, 'Big Dog')
    expect(input().value).toBe('Big Dog')
    button('Reset')!.click()
    await tick()
    await tick()
    expect(calls('set_nickname')[0][1]).toEqual({ serverId: 'S1', userId: 'U2', nickname: null })
  })

  it('is titled like the menu item that opens it (#137)', () => {
    const title = () => document.querySelector('[role=dialog]')!.getAttribute('aria-label')
    show({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(title()).toBe('Change server nickname')
    unmount(cmp!)
    show({ kind: 'server', userId: 'U1', serverId: 'S1' })
    expect(title()).toBe('Change your server nickname')
    unmount(cmp!)
    show({ kind: 'private', userId: 'U2' })
    expect(title()).toBe('Change nickname')
  })

  it('Clear shows only when you gave them a private nickname (#137: the only place to clear it)', () => {
    show({ kind: 'private', userId: 'U2' })
    expect(button('Clear')).toBeUndefined()
    unmount(cmp!)
    show({ kind: 'private', userId: 'U2' }, null, 'Sammy')
    expect(button('Clear')).toBeDefined()
    expect(button('Reset')).toBeUndefined()
  })

  it('a private nickname: only you see it', async () => {
    show({ kind: 'private', userId: 'U2' }, null, 'Sammy')
    expect(document.body.textContent).toContain('Only you see this name for Sam 🎸')
    button('Clear')!.click()
    await tick()
    await tick()
    expect(calls('set_private_nickname')).toEqual([['set_private_nickname', { userId: 'U2', nickname: null }]])
  })

  it('counts characters as the server does: 32 emoji fit, 33 characters do not', async () => {
    show({ kind: 'server', userId: 'U2', serverId: 'S1' })
    type('🎸'.repeat(32))
    expect(document.querySelector('[role=alert]')).toBeNull()
    expect(button('Save')!.disabled).toBe(false)
    type('x'.repeat(33))
    expect(document.querySelector('[role=alert]')!.textContent).toContain('1 to 32')
    expect(button('Save')!.disabled).toBe(true)
    await submit()
    expect(calls('set_nickname')).toHaveLength(0)
  })

  it('a refused rename says so and stays open', async () => {
    vi.mocked(invoke).mockImplementation((async () => { throw 'slow down a little, then try again' }) as never)
    show({ kind: 'server', userId: 'U2', serverId: 'S1' })
    type('Pup')
    await submit()
    await vi.waitFor(() => expect(document.querySelector('[role=alert]')?.textContent).toBe('slow down a little, then try again'))
    expect(onClose).not.toHaveBeenCalled()
  })
})
