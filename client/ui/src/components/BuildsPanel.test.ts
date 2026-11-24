// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Build } from '../lib/protocol/Build'
import type { BuildFacets } from '../lib/protocol/BuildFacets'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BuildsChannel from './BuildsChannel.svelte'
import { app, ui } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import { chooseOption, selectLabels, selectTrigger, selectValue } from './selectHarness'

const DAY = 86_400_000
const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    games: [
      { id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
      { id: 'G2', server_id: 'S1', name: 'Warframe', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
    ],
  })

const build = (id: string, b: Partial<Build> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: null, quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
})

const live = [build('M003'), build('M002', { game_id: 'G2' }), build('M001')]
const old = build('M000', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U1' })
const facets: Record<string, BuildFacets> = {
  false: { total: 3, games: [{ game_id: 'G1', count: 2 }, { game_id: 'G2', count: 1 }] },
  true: { total: 4, games: [{ game_id: 'G1', count: 3 }, { game_id: 'G2', count: 1 }] },
}
let bulk: (body: { game_id: string | null; before: string; dry_run: boolean }) => unknown = () => ({ builds: [], archived: 0 })

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const order = () => [...document.querySelectorAll<HTMLElement>('[data-build-id]')].map((e) => e.dataset.buildId)
const panel = () => document.querySelector<HTMLElement>('.builds-panel')
const gameRow = (name: string) => [...panel()!.querySelectorAll<HTMLButtonElement>('.games button')].find((b) => b.querySelector('.name')?.textContent === name)!
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)
const showArchived = () => panel()!.querySelector<HTMLInputElement>('input[type=checkbox]')!

beforeEach(() => {
  invoke.mockReset()
  try { localStorage.clear() } catch { /* none */ }
  bulk = () => ({ builds: [], archived: 0 })
  ui.notice = null
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const a = (args ?? {}) as Record<string, unknown>
    if (cmd === 'list_builds') return [...live, ...(a.archived ? [old] : [])].filter((m) => !a.game || m.build!.game_id === a.game)
    if (cmd === 'build_facets') return facets[String(a.archived)]
    if (cmd === 'bulk_archive_builds') return bulk(a.body as never)
    return undefined
  })
  app.state = applyReady(emptyState(), ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

async function show() {
  cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
  flushSync()
  await settle()
  flushSync()
}

describe('BuildsPanel', () => {
  it('clicking a game filters the view', async () => {
    await show()
    expect(panel()).not.toBeNull()
    const rows = [...panel()!.querySelectorAll<HTMLButtonElement>('.games button')].map((b) => [b.querySelector('.name')?.textContent, b.querySelector('.n')?.textContent])
    expect(rows).toEqual([['All games', '3'], ['The First Descendant', '2'], ['Warframe', '1']])
    expect(gameRow('All games').getAttribute('aria-pressed')).toBe('true')
    gameRow('Warframe').click()
    await settle()
    flushSync()
    expect(calls('list_builds').at(-1)![1]).toMatchObject({ game: 'G2' })
    expect(order()).toEqual(['M002'])
    expect(document.querySelector('.builds-head h2')?.textContent).toBe('Warframe')
    expect(document.querySelector('.builds-head .count')?.textContent).toBe('1 build')
    gameRow('All games').click()
    await settle()
    flushSync()
    expect(order()).toEqual(['M003', 'M002', 'M001'])
  })

  it('counts follow Show archived', async () => {
    await show()
    showArchived().click()
    await settle()
    flushSync()
    expect(calls('build_facets').at(-1)![1]).toEqual({ channelId: 'B1', archived: true })
    expect(gameRow('All games').querySelector('.n')?.textContent).toBe('4')
    expect(gameRow('The First Descendant').querySelector('.n')?.textContent).toBe('3')
    // the archived build shows, greyed with its badge
    expect(order()).toEqual(['M003', 'M002', 'M001', 'M000'])
    const card = document.querySelector<HTMLElement>('[data-build-id="M000"]')!
    expect(card.classList.contains('archived')).toBe(true)
    expect([...card.querySelectorAll('.badge')].map((b) => b.textContent)).toContain('ARCHIVED')
  })

  it('the bulk archive preview lists what it will archive before confirming', async () => {
    bulk = (b) => ({ builds: b.dry_run ? [{ message_id: 'M001', title: 'Build M001', game_id: 'G1', created_at: '2026-08-01T00:00:00Z' }, { message_id: 'M003', title: 'Build M003', game_id: 'G1', created_at: '2026-08-02T00:00:00Z' }] : [], archived: b.dry_run ? 0 : 2 })
    await show()
    const game = selectTrigger('Game to archive')!
    expect(selectLabels(game)).toEqual(['All games', 'The First Descendant', 'Warframe'])
    expect(selectValue(game)).toBe('')
    button('Archive matching builds…')!.click()
    await settle()
    flushSync()
    const body = (calls('bulk_archive_builds')[0][1] as { body: { game_id: string | null; before: string; dry_run: boolean } }).body
    expect(body.dry_run).toBe(true)
    expect(body.game_id).toBeNull()
    // 30 days ago by default
    expect(Math.abs(Date.parse(body.before) - (Date.now() - 30 * DAY))).toBeLessThan(60_000)
    const dialog = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dialog.textContent).toContain('2 builds')
    expect([...dialog.querySelectorAll('li')].map((l) => l.textContent?.trim())).toEqual(['Build M001', 'Build M003'])
    expect(calls('bulk_archive_builds')).toHaveLength(1) // nothing archived yet
  })

  it('the confirm sends the same filter without dry run', async () => {
    bulk = (b) => ({ builds: [{ message_id: 'M001', title: 'Build M001', game_id: 'G1', created_at: '2026-08-01T00:00:00Z' }], archived: b.dry_run ? 0 : 1 })
    await show()
    gameRow('The First Descendant').click()
    await settle()
    flushSync()
    // the dropdown follows the selected game; 90 days ago
    expect(selectValue(selectTrigger('Game to archive')!)).toBe('G1')
    button('90 days ago')!.click()
    flushSync()
    button('Archive matching builds…')!.click()
    await settle()
    flushSync()
    button('Archive 1 build')!.click()
    await settle()
    flushSync()
    const [dry, real] = calls('bulk_archive_builds').map((c) => (c[1] as { body: { game_id: string | null; before: string; dry_run: boolean } }).body)
    expect(dry).toMatchObject({ game_id: 'G1', dry_run: true })
    expect(Math.abs(Date.parse(dry.before) - (Date.now() - 90 * DAY))).toBeLessThan(60_000)
    expect(real).toEqual({ ...dry, dry_run: false })
    expect(document.querySelector('[role=dialog]')).toBeNull()
    expect(ui.notice).toBe('Archived 1 build')
  })

  it('a date picked archives builds saved before that day', async () => {
    bulk = () => ({ builds: [], archived: 0 })
    await show()
    chooseOption(selectTrigger('Game to archive')!, 'G2')
    const day = panel()!.querySelector<HTMLInputElement>('input[type=date]')!
    day.value = '2026-09-01'
    day.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    button('Archive matching builds…')!.click()
    await settle()
    flushSync()
    const body = (calls('bulk_archive_builds')[0][1] as { body: { game_id: string | null; before: string } }).body
    expect(body.game_id).toBe('G2')
    expect(body.before).toBe(new Date(2026, 8, 1).toISOString())
  })

  it("a non-owner's preview counts only their own builds (renders what the server returns)", async () => {
    bulk = () => ({ builds: [], archived: 0 })
    await show()
    button('Archive matching builds…')!.click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    const none = button('No builds match')!
    expect(none.disabled).toBe(true)
    // changing what it matches asks again
    button('90 days ago')!.click()
    flushSync()
    expect(button('Archive matching builds…')).toBeDefined()
  })

  it('the panel collapses and stays collapsed', async () => {
    await show()
    const toggle = document.querySelector<HTMLButtonElement>('.builds-head button[aria-label="Hide games panel"]')!
    // the toggle is the header's rightmost control
    expect(toggle.parentElement!.lastElementChild).toBe(toggle)
    toggle.click()
    flushSync()
    expect(panel()).toBeNull()
    unmount(cmp!)
    await show()
    expect(panel()).toBeNull()
    document.querySelector<HTMLButtonElement>('.builds-head button[aria-label="Show games panel"]')!.click()
    flushSync()
    expect(panel()).not.toBeNull()
  })
})
