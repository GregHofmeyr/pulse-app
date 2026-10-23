// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'

const ce = (id: string, server_id: string, name: string): CustomEmoji => ({
  id, server_id, name, hash: `h${id}`, still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00Z',
})

let cmp: ReturnType<typeof mount> | undefined
const onSend = vi.fn()
function show() {
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  app.state = {
    ...emptyState(), me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
    members: { S1: [{ user: me, nickname: null }], S2: [] },
    customEmoji: { '1': ce('1', 'S1', 'pog'), '2': ce('2', 'S2', 'pogchamp') },
  }
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {} },
  })
  flushSync()
  return document.querySelector('textarea')!
}
function type(box: HTMLTextAreaElement, value: string) {
  box.value = value
  box.setSelectionRange(value.length, value.length)
  box.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const press = (box: HTMLTextAreaElement, key: string) => {
  box.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  flushSync()
}
const options = () => [...document.querySelectorAll('[aria-label="Insert emoji"] [role=option]')].map((o) => o.textContent!.trim())

beforeEach(() => onSend.mockClear())
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('Composer emoji', () => {
  it('lists your servers\' custom emoji (with server name) and standard ones, at most 8', () => {
    const box = show()
    type(box, 'gg :po')
    const o = options()
    expect(o[0].replace(/\s+/g, ' ')).toBe(':pog: Main')
    expect(o.some((x) => x.includes('pogchamp'))).toBe(false) // not a member of S2
    type(box, 'gg :sm')
    expect(options()).toHaveLength(8)
  })

  it('arrows + Enter insert the pick; Esc dismisses; no popup after a URL colon', async () => {
    const box = show()
    type(box, ':po')
    press(box, 'ArrowDown')
    expect(document.querySelector('[aria-label="Insert emoji"] [aria-selected=true]')!.textContent).not.toContain('pog')
    press(box, 'ArrowUp')
    press(box, 'Enter')
    await tick()
    expect(box.value).toBe('<:pog:1> ')
    expect(onSend).not.toHaveBeenCalled()
    type(box, 'x :thumbsup')
    press(box, 'Escape')
    expect(options()).toHaveLength(0)
    type(box, 'see http://xy')
    expect(options()).toHaveLength(0)
  })

  it('the emoji button opens the picker and inserts at the caret', async () => {
    const box = show()
    type(box, 'ab')
    box.setSelectionRange(1, 1)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    const cell = document.querySelector('.picker button.cell[aria-label=":grinning:"]') as HTMLButtonElement
    cell.click()
    await tick()
    expect(box.value).toBe('a😀 b')
    expect(document.querySelector('.picker')).toBeNull()
  })
})
