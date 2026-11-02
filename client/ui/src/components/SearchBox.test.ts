// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => ({ hits: [], next_cursor: null, total: 0 }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import SearchBox from './SearchBox.svelte'
import { app } from '../lib/store.svelte'
import { closeSearch, search } from '../lib/search.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const user = (id: string, username: string) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
beforeEach(() => {
  invoke.mockClear()
  closeSearch()
  app.state = applyReady(emptyState(), testReady(
    [
      { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'C2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 },
      { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 2 },
      { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
    ],
    {
      members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: user('U2', 'sam'), nickname: null }, { user: user('U3', 'Amber'), nickname: null }] }],
      dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }],
      people: [{ user: user('U2', 'sam'), online: true, last_seen_at: null }],
    },
  ))
})

function show(serverId: string | null = 'S1', channelId = 'C1') {
  cmp = mount(SearchBox, { target: document.body, props: { serverId, channelId } })
  flushSync()
  return document.querySelector<HTMLInputElement>('input[role="combobox"]')!
}
const options = () => [...document.querySelectorAll<HTMLElement>('[role="listbox"] [role="option"]')]
/** Each option's text, without an avatar's initial. */
const optionText = () =>
  options().map((o) => {
    const c = o.cloneNode(true) as HTMLElement
    c.querySelectorAll('.av').forEach((a) => a.remove())
    return c.textContent!.replace(/\s+/g, ' ').trim()
  })
function type(input: HTMLInputElement, text: string) {
  input.value = text
  input.setSelectionRange(text.length, text.length)
  input.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
function key(input: HTMLInputElement, k: string, more: KeyboardEventInit = {}) {
  input.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...more }))
  flushSync()
}
const chips = () => [...document.querySelectorAll<HTMLElement>('.chip')].map((c) => c.textContent!.replace(/\s+/g, ' ').trim())

describe('SearchBox', () => {
  it('says where it searches', () => {
    expect(show().placeholder).toBe('Search Main')
    unmount(cmp!)
    cmp = undefined
    expect(show(null, 'D1').placeholder).toBe('Search this conversation')
  })

  it('focused and empty, it lists the filters with what each does', () => {
    const input = show()
    input.focus()
    flushSync()
    expect(input.getAttribute('aria-expanded')).toBe('true')
    expect(optionText()).toEqual([
      'from: user From a specific user',
      'mentions: user Mentions a specific user',
      'has: link, clip or quote Includes a link, a clip or a quote',
      'in: channel Sent in a specific channel',
    ])
    const list = document.querySelector('[role="listbox"]')!
    expect(input.getAttribute('aria-controls')).toBe(list.id)
    expect(list.classList.contains('menu-surface')).toBe(true)
  })

  it('offers no in: in a conversation', () => {
    const input = show(null, 'D1')
    input.focus()
    flushSync()
    expect(optionText().map((t) => t.split(':')[0])).toEqual(['from', 'mentions', 'has'])
  })

  it('arrows and Enter insert a filter and keep the focus in the box', async () => {
    const input = show()
    input.focus()
    flushSync()
    key(input, 'ArrowDown')
    expect(input.getAttribute('aria-activedescendant')).toBe(options()[0].id)
    expect(options()[0].getAttribute('aria-selected')).toBe('true')
    key(input, 'Enter')
    await tick()
    expect(input.value).toBe('from:')
    expect(document.activeElement).toBe(input)
    // Now it suggests people: everyone, A to Z.
    expect(optionText()).toEqual(['alex', 'Amber', 'sam'])
  })

  it('suggests members matching any part of the name, prefix matches first', () => {
    const input = show()
    input.focus()
    type(input, 'from:am')
    expect(optionText()).toEqual(['Amber', 'sam'])
    // The first suggestion is ready for Enter.
    expect(options()[0].getAttribute('aria-selected')).toBe('true')
  })

  it('suggests channels for in: and the fixed list for has:', () => {
    const input = show()
    input.focus()
    type(input, 'in:me')
    expect(optionText()).toEqual(['memes'])
    type(input, 'has:')
    expect(optionText().map((t) => t.split(' ')[0])).toEqual(['link', 'clip', 'quote'])
  })

  it('picking a suggestion makes a chip and goes back to the filters', async () => {
    const input = show()
    input.focus()
    type(input, 'pizza from:sa')
    options()[0].dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
    flushSync()
    await tick()
    expect(chips()).toEqual(['from: sam'])
    expect(input.value).toBe('pizza ')
    expect(document.activeElement).toBe(input)
    expect(optionText()[0]).toContain('from: user')
  })

  it('Backspace in an empty box removes the last chip; ✕ removes one', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter')
    type(input, 'has:cl')
    key(input, 'Tab')
    await tick()
    expect(chips()).toEqual(['from: sam', 'has: clip'])
    type(input, '')
    key(input, 'Backspace')
    expect(chips()).toEqual(['from: sam'])
    document.querySelector<HTMLButtonElement>('.chip button[aria-label="Remove from: sam"]')!.click()
    flushSync()
    expect(chips()).toEqual([])
  })

  it('Enter with no suggestion picked runs the search with structured parameters', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter') // picks sam
    type(input, 'pizza')
    key(input, 'Enter')
    await tick()
    expect(invoke).toHaveBeenCalledWith('search_messages', {
      serverId: 'S1', channelId: null,
      query: { q: 'pizza', from: 'U2', in_channel: null, has: [], mentions: null, cursor: null },
    })
    expect(search.open).toBe(true)
  })

  it('a filter typed in full becomes a chip when the search runs', async () => {
    const input = show()
    input.focus()
    type(input, 'pizza in:#general mentions:SAM')
    key(input, 'Escape') // the list closes; the text stays
    key(input, 'Enter')
    await tick()
    expect(invoke).toHaveBeenCalledWith('search_messages', expect.objectContaining({
      query: { q: 'pizza', from: null, in_channel: 'C1', has: [], mentions: 'U2', cursor: null },
    }))
    expect(chips()).toEqual(['in: #general', 'mentions: sam'])
    expect(input.value).toBe('pizza')
  })

  it('an empty search does nothing', () => {
    const input = show()
    input.focus()
    key(input, 'Enter')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('Ctrl+F focuses the box', () => {
    const input = show()
    const e = new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true })
    window.dispatchEvent(e)
    flushSync()
    expect(document.activeElement).toBe(input)
    expect(e.defaultPrevented).toBe(true)
  })

  it('Escape closes the list, then the results', async () => {
    const input = show()
    input.focus()
    type(input, 'pizza')
    key(input, 'Enter')
    await tick()
    expect(search.open).toBe(true)
    key(input, 'Escape')
    expect(search.open).toBe(false)
  })
})
