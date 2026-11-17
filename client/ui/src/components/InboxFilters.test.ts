// @vitest-environment jsdom
// Custom inbox filters (#307): the rule, the row of saved filters under All and Mentions, fetching through the picked
// one, live items it doesn't show left out, the editor (new, edit, delete) and Mark all read through a filter.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InboxItem } from '../lib/protocol/InboxItem'
import type { InboxFilter } from '../lib/protocol/InboxFilter'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import InboxPopover from './InboxPopover.svelte'
import { closeInbox, inbox, noteInboxEvent, openInbox, setFilter } from '../lib/inbox.svelte'
import { filterProblem, matchesFilter } from '../lib/inbox'
import { app, clock, handleReady, resetState } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'
import { testItem as item } from '../lib/testInbox'

const sam = { id: 'SAM', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'KIM', username: 'kim', display_name: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const NOW = Date.parse('2026-11-12T10:05:00.000Z')
const samOnly: InboxFilter = { name: 'Sam', people: ['SAM'], channels: [], unread_only: true }

describe('the rule', () => {
  const a = item('M1', {}, { author_id: 'SAM', channel_id: 'C1' })
  it('people and chats (either empty: any), and unread only', () => {
    expect(matchesFilter(a, null)).toBe(true)
    expect(matchesFilter(a, samOnly)).toBe(true)
    expect(matchesFilter({ ...a, read: true }, samOnly)).toBe(false)
    expect(matchesFilter(a, { ...samOnly, people: ['KIM'] })).toBe(false)
    expect(matchesFilter(a, { ...samOnly, people: [], channels: ['C2'] })).toBe(false)
    expect(matchesFilter(a, { ...samOnly, channels: ['C1'] })).toBe(true)
  })
  it('a name, unique ignoring case, and something to narrow by', () => {
    expect(filterProblem({ ...samOnly, name: ' ' }, [])).toBe('Give it a name.')
    expect(filterProblem(samOnly, [{ ...samOnly, name: 'SAM' }])).toContain('already')
    expect(filterProblem({ ...samOnly, people: [], unread_only: false }, [])).toContain('Pick')
    expect(filterProblem(samOnly, [])).toBe('')
  })
})

let cmp: ReturnType<typeof mount> | undefined
let saved: InboxFilter[] = []
const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); flushSync() }
async function show() {
  openInbox()
  cmp = mount(InboxPopover, { target: document.body, props: { onClose: () => {} } })
  flushSync()
  await settle()
}
const chips = () => [...document.querySelectorAll<HTMLButtonElement>('.filters .chip')]
const chip = (name: string) => chips().find((c) => c.textContent?.trim() === name)!
const rows = () => [...document.querySelectorAll<HTMLButtonElement>('.item')]
const inboxCalls = () => invoke.mock.calls.filter(([c]) => c === 'inbox').map(([, a]) => a)
const fromSam = (id: string, over: Partial<InboxItem> = {}) => item(id, over, { author_id: 'SAM', content: `${id} from sam`, mentions: ['U1'] })
const fromKim = (id: string) => item(id, {}, { author_id: 'KIM', content: `${id} from kim`, mentions: ['U1'] })

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  clock.nowIso = new Date(NOW).toISOString()
  localStorage.clear()
  resetState()
  closeInbox()
  inbox.tab = 'all'
  inbox.filter = null
  saved = [samOnly]
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const a = args as { filter?: InboxFilter; filters?: InboxFilter[] } | undefined
    if (cmd === 'inbox') return a?.filter ? { items: [fromSam('M3')], unread: 2 } : { items: [fromSam('M3'), fromKim('M2')], unread: 2 }
    if (cmd === 'save_inbox_filters') return (saved = a!.filters!)
    if (cmd === 'inbox_unread') return 0
    return undefined
  })
  handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } as never], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] }],
    inbox_unread: 2,
    inbox_filters: [samOnly],
  }))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  inbox.filter = null
  vi.useRealTimers()
})

describe('the filter row', () => {
  it('Everything, then your filters; picking one fetches through it, remembered on this device', async () => {
    await show()
    expect(chips().map((c) => c.textContent?.trim())).toEqual(['Everything', 'Sam', 'New filter'])
    expect(chip('Everything').getAttribute('aria-pressed')).toBe('true')
    expect(rows()).toHaveLength(2)
    chip('Sam').click()
    await settle()
    expect(inboxCalls().at(-1)).toEqual({ before: null, mentions: false, filter: samOnly })
    expect(chip('Sam').getAttribute('aria-pressed')).toBe('true')
    expect(rows()).toHaveLength(1)
    expect(localStorage.getItem('pulse.inbox.filter')).toBe('Sam')
  })

  it('a live item the filter doesn’t show stays out; one it shows comes in', async () => {
    setFilter('Sam')
    await show()
    noteInboxEvent({ t: 'InboxItemAdded', d: { user_id: 'U1', item: fromKim('M9'), unread: 3 } })
    noteInboxEvent({ t: 'InboxItemAdded', d: { user_id: 'U1', item: fromSam('M8'), unread: 4 } })
    flushSync()
    expect(rows().map((r) => r.querySelector('.said')?.textContent)).toEqual(['M8 from sam', 'M3 from sam'])
  })

  it('Mark all read through a filter reads only what it shows, one by one', async () => {
    setFilter('Sam')
    await show()
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Mark all read')!.click()
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'inbox_read').map(([, a]) => a)).toEqual([{ messageId: 'M3' }])
    expect(invoke.mock.calls.some(([c]) => c === 'inbox_read_all')).toBe(false)
  })
})

describe('the editor', () => {
  it('New filter: name it, tick people, save; it is picked', async () => {
    await show()
    chip('New filter').click()
    flushSync()
    const form = document.querySelector<HTMLFormElement>('form.editor')!
    const name = form.querySelector<HTMLInputElement>('input[aria-label="Filter name"]')!
    name.value = 'Kim'
    name.dispatchEvent(new Event('input', { bubbles: true }))
    const kimBox = [...form.querySelectorAll('li label')].find((l) => l.textContent?.includes('kim'))!.querySelector<HTMLInputElement>('input')!
    kimBox.click()
    flushSync()
    form.querySelector<HTMLButtonElement>('button[type=submit]')!.click()
    await settle()
    expect(saved).toEqual([samOnly, { name: 'Kim', people: ['KIM'], channels: [], unread_only: true }])
    expect(app.state.inboxFilters).toEqual(saved)
    expect(inbox.filter).toBe('Kim')
    expect(document.querySelector('form.editor')).toBeNull()
  })

  it('a clash is refused in place; Delete removes the picked one and goes back to Everything', async () => {
    setFilter('Sam')
    await show()
    document.querySelector<HTMLButtonElement>('[aria-label="Edit filter Sam"]')!.click()
    flushSync()
    const form = document.querySelector<HTMLFormElement>('form.editor')!
    ;[...form.querySelectorAll('button')].find((b) => b.textContent === 'Delete')!.click()
    await settle()
    expect(saved).toEqual([])
    expect(inbox.filter).toBeNull()
    expect(chips().map((c) => c.textContent?.trim())).toEqual(['Everything', 'New filter'])
  })
})
