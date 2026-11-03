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

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })

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

function show(serverId: string | null = 'S1', channelId = 'C1', compact = false) {
  cmp = mount(SearchBox, { target: document.body, props: { serverId, channelId, compact } })
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

describe('SearchBox in a narrow header (#97)', () => {
  const box = () => document.querySelector<HTMLElement>('.search')!
  const mag = () => document.querySelector<HTMLButtonElement>('button.mag')

  it('folds to a magnifier button; clicking it opens the box with the focus in it', async () => {
    const input = show('S1', 'C1', true)
    expect(box().classList.contains('collapsed')).toBe(true)
    expect(mag()?.getAttribute('aria-label')).toBe('Search')
    mag()!.click()
    flushSync()
    await tick()
    expect(document.activeElement).toBe(input)
    expect(box().classList.contains('collapsed')).toBe(false)
    expect(mag()).toBeNull()
  })

  it('folds back when you leave it empty, and stays open while it holds words or a filter', async () => {
    const input = show('S1', 'C1', true)
    input.focus()
    flushSync()
    input.blur()
    flushSync()
    expect(box().classList.contains('collapsed')).toBe(true)
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter')
    await tick()
    input.blur()
    flushSync()
    expect(chips()).toEqual(['from: sam'])
    expect(box().classList.contains('collapsed')).toBe(false)
  })

  it('Ctrl+F opens it too', () => {
    const input = show('S1', 'C1', true)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement).toBe(input)
    expect(box().classList.contains('collapsed')).toBe(false)
  })

  it('in a wide header there is no magnifier button and the box is always open', () => {
    show('S1', 'C1', false)
    expect(mag()).toBeNull()
    expect(box().classList.contains('collapsed')).toBe(false)
  })
})

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

  it('from: and mentions: find people by username, display name, nickname or your private nickname, shown as you see them (#47, #48)', () => {
    const sam = { ...user('U2', 'sam'), display_name: 'Samwise' }
    app.state = {
      ...app.state,
      people: { U2: { user: sam, online: true, last_seen_at: null } },
      members: { S1: [{ user: testMe, nickname: null }, { user: sam, nickname: 'Gardener' }, { user: user('U3', 'Amber'), nickname: null }] },
      privateNicks: { U3: 'Bestie' },
    }
    const input = show()
    input.focus()
    for (const q of ['sam', 'wise', 'garden']) {
      type(input, `from:${q}`)
      expect(optionText(), q).toEqual(['Gardener'])
    }
    type(input, 'mentions:best')
    expect(optionText()).toEqual(['Bestie'])
    type(input, 'mentions:amb')
    expect(optionText()).toEqual(['Bestie'])
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

  it('after a click picks a name, Enter searches: the filter list now under the still pointer stays unpicked', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sa')
    const at = { clientX: 40, clientY: 60, bubbles: true, cancelable: true }
    options()[0].dispatchEvent(new MouseEvent('mousemove', at))
    options()[0].dispatchEvent(new MouseEvent('mousedown', at))
    flushSync()
    await tick()
    expect(chips()).toEqual(['from: sam'])
    // the browser sends a mousemove when the list changes under a pointer that didn't move
    options()[0].dispatchEvent(new MouseEvent('mousemove', at))
    flushSync()
    expect(options().some((o) => o.classList.contains('hi'))).toBe(false)
    key(input, 'Enter')
    await tick()
    expect(chips()).toEqual(['from: sam'])
    expect(invoke).toHaveBeenCalledWith('search_messages', {
      serverId: 'S1', channelId: null,
      query: { q: '', from: 'U2', in_channel: null, has: [], mentions: null, cursor: null },
    })
  })

  it('with a chip in the box the placeholder goes (it would spill out); the field keeps its name', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter')
    await tick()
    expect(chips()).toEqual(['from: sam'])
    expect(input.placeholder).toBe('')
    expect(input.getAttribute('aria-label')).toBe('Search Main')
  })

  it('hovering a row (a real pointer move) never makes it the one Enter picks: Enter searches', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter') // picks sam: the filter rows come back
    await tick()
    options()[1].dispatchEvent(new MouseEvent('mousemove', { clientX: 10, clientY: 10, bubbles: true }))
    options()[1].dispatchEvent(new MouseEvent('mousemove', { clientX: 12, clientY: 14, bubbles: true }))
    flushSync()
    expect(options().some((o) => o.classList.contains('hi'))).toBe(false)
    expect(input.getAttribute('aria-activedescendant')).toBeNull()
    key(input, 'Enter')
    await tick()
    expect(input.value).toBe('')
    expect(invoke).toHaveBeenCalledWith('search_messages', expect.objectContaining({ query: expect.objectContaining({ from: 'U2' }) }))
  })

  it('a click still picks the row under the pointer', async () => {
    const input = show()
    input.focus()
    type(input, '')
    options()[1].dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
    flushSync()
    await tick()
    expect(input.value).toMatch(/^\w+:$/)
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

  it('Escape closes the list, then the results, then clears the box, then leaves it', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter')
    type(input, 'pizza')
    key(input, 'Enter')
    await tick()
    expect(search.open).toBe(true)
    input.focus()
    flushSync()
    key(input, 'Escape')
    expect(search.open).toBe(false)
    expect(input.value).toBe('pizza')
    key(input, 'Escape')
    expect(input.value).toBe('')
    expect(chips()).toEqual([])
    expect(document.activeElement).toBe(input)
    key(input, 'Escape') // the filter list it reopened on the empty box
    key(input, 'Escape')
    expect(document.activeElement).not.toBe(input)
  })

  it('a click into another channel keeps what was searched in the box', async () => {
    const input = show()
    input.focus()
    type(input, 'from:sam')
    key(input, 'Enter')
    type(input, 'pizza')
    key(input, 'Enter')
    await tick()
    // The channel view (and its box) is rebuilt for the result's channel.
    unmount(cmp!)
    cmp = undefined
    const again = show('S1', 'C2')
    expect(again.value).toBe('pizza')
    expect(chips()).toEqual(['from: sam'])
  })

  it('starts empty when the open results belong to another scope', () => {
    Object.assign(search, { open: true, scope: { kind: 'conversation', channelId: 'D1' }, words: 'pizza', chips: [] })
    expect(show().value).toBe('')
  })

  it('emoji or punctuation alone says to type a word instead of searching', () => {
    const input = show()
    input.focus()
    type(input, '🔥 ?') // no filter starts with "?": the list is closed
    key(input, 'Enter')
    expect(invoke).not.toHaveBeenCalled()
    expect(document.querySelector('.hint')!.textContent).toContain('Type a word or pick a filter')
    type(input, '🔥 pizza')
    expect(document.querySelector('.hint')?.textContent ?? '').toBe('')
  })

  it('a person typed with @ still becomes a chip', async () => {
    const input = show()
    input.focus()
    type(input, 'from:@sam pizza') // the list is closed on "pizza"
    key(input, 'Enter')
    await tick()
    expect(chips()).toEqual(['from: sam'])
  })

  it('aria-controls names the list only while there is one', () => {
    const input = show()
    expect(input.hasAttribute('aria-controls')).toBe(false)
    input.focus()
    flushSync()
    expect(document.getElementById(input.getAttribute('aria-controls')!)?.getAttribute('role')).toBe('listbox')
  })

  it('Ctrl+F leaves an open dialog alone', () => {
    const input = show()
    const dialog = document.createElement('div')
    dialog.setAttribute('aria-modal', 'true')
    document.body.append(dialog)
    const e = new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true })
    window.dispatchEvent(e)
    expect(document.activeElement).not.toBe(input)
    expect(e.defaultPrevented).toBe(false)
  })
})
