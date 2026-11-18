// @vitest-environment jsdom
// The composer's + menu (#461): the bar keeps only +, the message box and the emoji/GIF button. The + menu holds
// Attach files, a divider, Picker wheel, Poll, Plan something, Invite to game, a divider and Send later…, each only
// where it applies; the arrow keys move in it and Escape closes it, back on the + button.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { setPlanSomething } from '../lib/planPrefs.svelte'
import { plusButton, plusItem, plusRows } from './plusMenuHarness'

let cmp: ReturnType<typeof mount> | undefined
const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const playing = { name: 'Space Game', steam_app_id: 570, started_at: null, art: null, join: null }
const everything = { files: true, wheelChannel: 'C1', pollChannel: 'C1', planChannel: 'C1', inviteChannel: 'C1', onSchedule: async () => {}, onSendGif: () => {} }

function show(extra: Record<string, unknown> = {}, gaming = false) {
  app.state = { ...emptyState(), me } as never
  if (gaming) app.state = { ...app.state, people: { U1: { playing } } } as never
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend: () => {}, onTyping: () => {}, ...extra },
  })
  flushSync()
}
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Add to your message"]')
const settle = async () => { for (let i = 0; i < 4; i++) await Promise.resolve(); flushSync() }

beforeEach(() => setPlanSomething(true))
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Composer + menu (#461)', () => {
  it('the bar keeps only +, the box and one emoji/GIF button', () => {
    show(everything, true)
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('.wrap > button')].map((b) => b.getAttribute('aria-label'))
    expect(buttons).toEqual(['More', 'Emoji and GIFs'])
  })

  it('holds Attach files, a divider, the four tools, a divider, then Send later…', () => {
    show(everything, true)
    plusButton()!.click()
    flushSync()
    expect(plusRows()).toEqual(['Attach files', '---', 'Picker wheel', 'Poll', 'Plan something', 'Invite to game', '---', 'Send later…'])
    expect(plusButton()!.getAttribute('aria-expanded')).toBe('true')
  })

  it('offers only what applies here: no game invite out of a game, no plan with Plan something off', () => {
    setPlanSomething(false)
    show(everything)
    expect(plusItem('invite')).toBeNull()
    expect(plusItem('plan')).toBeNull()
    expect(plusRows()).toEqual(['Attach files', '---', 'Picker wheel', 'Poll', '---', 'Send later…'])
  })

  it('no + button where nothing applies (a thread)', () => {
    show({})
    expect(plusButton()).toBeNull()
  })

  it('a thread with files only: just Attach files, no divider', () => {
    show({ files: true })
    expect(plusRows()).toEqual([])
    plusButton()!.click()
    flushSync()
    expect(plusRows()).toEqual(['Attach files'])
  })

  it('the first row takes focus; arrows move and wrap; Escape closes back on the + button', async () => {
    show(everything, true)
    plusButton()!.click()
    flushSync()
    await settle()
    const rows = [...menu()!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')]
    expect(document.activeElement).toBe(rows[0])
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    expect(document.activeElement).toBe(rows[1])
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }))
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }))
    expect(document.activeElement).toBe(rows[rows.length - 1])
    menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(plusButton())
  })

  it('Invite to game sends the invite and closes the menu', async () => {
    show(everything, true)
    plusItem('invite')!.click()
    flushSync()
    await settle()
    expect(menu()).toBeNull()
    expect(invoke.mock.calls.some(([cmd]) => cmd === 'send_game_invite')).toBe(true)
  })
})
