// @vitest-environment jsdom
// Your conversation list's pins and categories (#305): Pinned on top, your categories (fold from the heading, a
// count when folded), then the rest; the conversation's right-click menu pins, moves into a category or a new one;
// a category's right-click menu renames or deletes it; another device's change (DmLayoutChanged) shows at once.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'
import type { DmLayout } from '../lib/protocol/DmLayout'

let answer: DmLayout = { categories: [], placements: [] }
const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => answer)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ConversationList from './ConversationList.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { categoryNameOk, dmSections } from '../lib/dmLayout'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const [alex, sam, kim, jo] = [user('U1', 'alex'), user('U2', 'sam'), user('U3', 'kim'), user('U4', 'jo')]

let cmp: ReturnType<typeof mount> | undefined
function show(layout: DmLayout, activeId: string | null = null) {
  app.state = {
    ...emptyState(), me: alex,
    people: Object.fromEntries([alex, sam, kim, jo].map((u) => [u.id, { user: u, online: true, last_seen_at: null }])),
    channels: {
      D1: { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
      D2: { id: 'D2', server_id: null, kind: 'dm', name: null, position: 0 },
      G1: { id: 'G1', server_id: null, kind: 'group', name: 'squad', position: 0 },
    },
    dmMembers: { D1: ['U1', 'U2'], D2: ['U1', 'U4'], G1: ['U1', 'U2', 'U3'] },
    reads: { G1: { channel_id: 'G1', last_read_message_id: null, unread: 3, mentions: 0 } },
    dmLayout: layout,
  } as never
  cmp = mount(ConversationList, { target: document.body, props: { activeId, onOpen: () => {}, onNew: () => {}, onLeft: () => {} } })
  flushSync()
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
beforeEach(() => {
  resetState()
  invoke.mockClear()
  answer = { categories: [], placements: [] }
})

/** The list top to bottom: headings as `# NAME`, rows as their names. */
const outline = () => [...document.querySelectorAll<HTMLElement>('.list > .label, .list > .row')]
  .map((el) => (el.classList.contains('label') ? `# ${el.textContent!.replace(/\s+/g, ' ').trim()}` : el.querySelector('.name')!.textContent!.trim()))
  .filter((t) => t !== 'Notes')
const rowFor = (text: string) => [...document.querySelectorAll<HTMLElement>('.list .row')].find((r) => r.querySelector('.name')?.textContent?.includes(text))!
const rightClick = (el: Element) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
  flushSync()
}
const item = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role=menuitem], [role=menu] [role=menuitemradio]')].find((b) => b.textContent?.trim() === text)
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }
const FRIENDS = { id: 'K1', name: 'Friends', collapsed: false }

describe('pins and categories (#305)', () => {
  it('without any, the list is as it was: no headings', () => {
    show({ categories: [], placements: [] })
    expect(outline().sort()).toEqual(['jo', 'sam', 'squad'])
    expect(document.querySelector('.list > .label')).toBeNull()
  })

  it('Pinned first, then each category, then the rest under Conversations', () => {
    show({ categories: [FRIENDS], placements: [{ channel_id: 'D2', pinned: true, category_id: null }, { channel_id: 'G1', pinned: false, category_id: 'K1' }] })
    expect(outline()).toEqual(['# Pinned', 'jo', '# Friends', 'squad', '# Conversations', 'sam'])
  })

  it('folding a category hides its rows, shows its count and tells the server', async () => {
    show({ categories: [FRIENDS], placements: [{ channel_id: 'G1', pinned: false, category_id: 'K1' }] })
    answer = { categories: [{ ...FRIENDS, collapsed: true }], placements: [{ channel_id: 'G1', pinned: false, category_id: 'K1' }] }
    document.querySelector<HTMLButtonElement>('.label .section[aria-expanded]')!.click()
    flushSync()
    expect(outline().slice(0, 2)).toEqual(['# Friends 3', '# Conversations'])
    expect(outline()).not.toContain('squad')
    expect(invoke).toHaveBeenCalledWith('update_dm_category', { id: 'K1', request: { collapsed: true } })
    await flush()
  })

  it('right-click a conversation: Pin to top, and Move to category into one of yours', async () => {
    show({ categories: [FRIENDS], placements: [] })
    rightClick(rowFor('sam'))
    answer = { categories: [FRIENDS], placements: [{ channel_id: 'D1', pinned: true, category_id: null }] }
    item('Pin to top')!.click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('place_dm', { channelId: 'D1', request: { pinned: true, category_id: null } })
    expect(outline().slice(0, 2)).toEqual(['# Pinned', 'sam'])
    rightClick(rowFor('sam'))
    expect(item('Unpin')).toBeDefined()
    item('Move to category')!.click()
    flushSync()
    item('Friends')!.click()
    await flush()
    expect(invoke).toHaveBeenLastCalledWith('place_dm', { channelId: 'D1', request: { pinned: true, category_id: 'K1' } })
  })

  it('New category… names one and moves the conversation into it', async () => {
    show({ categories: [], placements: [] })
    rightClick(rowFor('jo'))
    item('Move to category')!.click()
    flushSync()
    item('New category…')!.click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('[role=menu] input[aria-label="Category name"]')!
    field.value = 'Work'
    field.dispatchEvent(new Event('input'))
    flushSync()
    answer = { categories: [{ id: 'K9', name: 'Work', collapsed: false }], placements: [] }
    item('Create and move')!.click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('create_dm_category', { name: 'Work' })
    expect(invoke).toHaveBeenLastCalledWith('place_dm', { channelId: 'D2', request: { pinned: false, category_id: 'K9' } })
  })

  it("a category's right-click menu renames it in its heading, or deletes it", async () => {
    show({ categories: [FRIENDS], placements: [{ channel_id: 'G1', pinned: false, category_id: 'K1' }] })
    rightClick(document.querySelector('.label .section[aria-expanded]')!)
    expect([...document.querySelectorAll('[role=menu] [role=menuitem]')].map((b) => b.textContent!.trim())).toEqual(['Rename category…', 'Delete category'])
    answer = { categories: [{ ...FRIENDS, name: 'Mates' }], placements: [{ channel_id: 'G1', pinned: false, category_id: 'K1' }] }
    item('Rename category…')!.click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('input[aria-label="Rename category Friends"]')!
    field.value = 'Mates'
    field.dispatchEvent(new Event('input'))
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await flush()
    expect(invoke).toHaveBeenCalledWith('update_dm_category', { id: 'K1', request: { name: 'Mates' } })
    rightClick(document.querySelector('.label .section[aria-expanded]')!)
    item('Delete category')!.click()
    await flush()
    expect(invoke).toHaveBeenLastCalledWith('delete_dm_category', { id: 'K1' })
  })

  it('a category moves up and down from its menu, only where it can go (#419)', async () => {
    const MATES = { id: 'K2', name: 'Mates', collapsed: false }
    const placements = [{ channel_id: 'G1', pinned: false, category_id: 'K1' }, { channel_id: 'D2', pinned: false, category_id: 'K2' }]
    show({ categories: [FRIENDS, MATES], placements })
    const headings = () => [...document.querySelectorAll<HTMLElement>('.label .section[aria-expanded]')]
    const menuItems = () => [...document.querySelectorAll('[role=menu] [role=menuitem]')].map((b) => b.textContent!.trim())
    rightClick(headings()[0])
    expect(menuItems()).toEqual(['Rename category…', 'Move down', 'Delete category'])
    answer = { categories: [MATES, FRIENDS], placements }
    item('Move down')!.click()
    await flush()
    expect(invoke).toHaveBeenLastCalledWith('update_dm_category', { id: 'K1', request: { position: 1 } })
    expect(outline().indexOf('# Mates')).toBeLessThan(outline().indexOf('# Friends'))
    rightClick(headings()[1])
    expect(menuItems()).toEqual(['Rename category…', 'Move up', 'Delete category'])
    // refused: put back
    invoke.mockImplementationOnce(async () => { throw new Error('nope') })
    item('Move up')!.click()
    await flush()
    expect(outline().indexOf('# Mates')).toBeLessThan(outline().indexOf('# Friends'))
  })

  it("a menu entry's name can't be a category's (#419)", async () => {
    show({ categories: [], placements: [] })
    rightClick(rowFor('jo'))
    item('Move to category')!.click()
    flushSync()
    item('New category…')!.click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('[role=menu] input[aria-label="Category name"]')!
    field.value = 'remove from category'
    field.dispatchEvent(new Event('input'))
    flushSync()
    expect(item('Create and move')!.disabled).toBe(true)
    expect(document.querySelector('[role=menu] .taken')?.textContent).toBe("That's a menu entry: pick another name")
  })

  it("another device's change shows at once", () => {
    show({ categories: [], placements: [] })
    app.state = applyEvent(app.state, { t: 'DmLayoutChanged', d: { user_id: 'U1', layout: { categories: [], placements: [{ channel_id: 'G1', pinned: true, category_id: null }] } } }, Date.now())
    flushSync()
    expect(outline().slice(0, 2)).toEqual(['# Pinned', 'squad'])
  })
})

describe('dmSections', () => {
  const c = (id: string, notes = false) => ({ channel: { id }, notes })
  it('pinned wins over a category; an unknown category falls back to the rest; notes stay apart', () => {
    const s = dmSections([c('N', true), c('A'), c('B'), c('C')], {
      categories: [{ id: 'K', name: 'k', collapsed: false }],
      placements: [{ channel_id: 'A', pinned: true, category_id: 'K' }, { channel_id: 'B', pinned: false, category_id: 'K' }, { channel_id: 'C', pinned: false, category_id: 'GONE' }, { channel_id: 'N', pinned: true, category_id: null }],
    })
    expect(s.notes.map((x) => x.channel.id)).toEqual(['N'])
    expect(s.pinned.map((x) => x.channel.id)).toEqual(['A'])
    expect(s.categories[0].items.map((x) => x.channel.id)).toEqual(['B'])
    expect(s.rest.map((x) => x.channel.id)).toEqual(['C'])
  })
})

describe('categoryNameOk (#419)', () => {
  it('refuses the Move flyout\'s own entries, ignoring case and three dots for the ellipsis', () => {
    for (const n of ['New category…', ' new category... ', 'REMOVE FROM CATEGORY']) expect(categoryNameOk(n)).toBe(false)
    expect(categoryNameOk('New category')).toBe(true)
    expect(categoryNameOk('Friends')).toBe(true)
  })
})
