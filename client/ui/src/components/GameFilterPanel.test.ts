// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { createRawSnippet, flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import GameFilterPanel from './GameFilterPanel.svelte'
import FilterList from './FilterList.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { reactiveProps } from '../lib/testProps.svelte'
import { testReady } from '../lib/testReady'

const ready = () =>
  testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
    games: [
      { id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
      { id: 'G2', server_id: 'S1', name: 'Rocket League', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
    ],
  })

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  app.state = applyReady(emptyState(), ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const rows = () =>
  [...document.querySelectorAll<HTMLButtonElement>('.games button')].map((b) => [b.querySelector('.name')?.textContent, b.querySelector('.n')?.textContent])
const row = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.games button')].find((b) => b.querySelector('.name')?.textContent === name)!

function panel(over: Record<string, unknown> = {}) {
  const p = reactiveProps({
    serverId: 'S1',
    games: [{ game_id: 'G2', count: 4 }, { game_id: 'G9', count: 3 }, { game_id: 'G1', count: 2 }] as { game_id: string; count: number }[] | undefined,
    total: 7 as number | null | undefined,
    selected: undefined as string | undefined,
    onSelect: vi.fn(),
    ...over,
  })
  cmp = mount(GameFilterPanel, { target: document.body, props: p })
  flushSync()
  return p
}

describe('GameFilterPanel (#663)', () => {
  it('"All games" with the total first, then each game the registry knows, in the given order, with its count', () => {
    panel()
    // G9 isn't in the registry yet: no row until it arrives
    expect(rows()).toEqual([['All games', '7'], ['Rocket League', '4'], ['Apex', '2']])
    const aside = document.querySelector('aside')!
    expect(aside.getAttribute('aria-label')).toBe('Games')
    expect(aside.querySelector('h3.menu-label')?.textContent).toBe('Games')
  })

  it('the selected game is pressed (and only it); a click picks, "All games" clears', () => {
    const p = panel()
    expect(row('All games').getAttribute('aria-pressed')).toBe('true')
    expect(row('Apex').getAttribute('aria-pressed')).toBe('false')
    row('Apex').click()
    expect(p.onSelect).toHaveBeenLastCalledWith('G1')
    p.selected = 'G1'
    flushSync()
    expect(row('Apex').getAttribute('aria-pressed')).toBe('true')
    expect(row('All games').getAttribute('aria-pressed')).toBe('false')
    row('All games').click()
    expect(p.onSelect).toHaveBeenLastCalledWith(undefined)
  })

  it('a picked game whose row is gone keeps one, with count 0, still pressed (a game the registry lacks reads "Game")', () => {
    const p = panel({ selected: 'G1', games: [{ game_id: 'G2', count: 4 }] })
    expect(rows()).toEqual([['All games', '7'], ['Rocket League', '4'], ['Apex', '0']])
    expect(row('Apex').getAttribute('aria-pressed')).toBe('true')
    p.selected = 'G7'
    flushSync()
    expect(rows()).toEqual([['All games', '7'], ['Rocket League', '4'], ['Game', '0']])
    expect(row('Game').getAttribute('aria-pressed')).toBe('true')
    // counts not loaded yet: the row shows, without a number
    p.games = undefined
    p.total = undefined
    flushSync()
    expect(rows()).toEqual([['All games', ''], ['Game', '']])
  })

  it('no counts yet (or a total the server did not send): "All games" shows no number, never a guess', () => {
    panel({ games: undefined, total: undefined })
    expect(rows()).toEqual([['All games', '']])
  })

  it('the rows are one labelled group, each a plain button', () => {
    panel()
    const group = document.querySelector<HTMLElement>('.games')!
    expect(group.getAttribute('role')).toBe('group')
    const label = document.getElementById(group.getAttribute('aria-labelledby')!)
    expect(label?.textContent).toBe('Games')
    for (const b of group.querySelectorAll('button')) expect(b.getAttribute('type')).toBe('button')
  })

  it('further sections come after the games, behind a separator, and its own class is kept', () => {
    const extra = createRawSnippet(() => ({ render: () => '<section class="extra">More</section>' }))
    panel({ class: 'builds-panel', children: extra })
    const aside = document.querySelector('aside')!
    expect(aside.classList.contains('builds-panel')).toBe(true)
    const kids = [...aside.children]
    expect(kids.map((k) => k.tagName.toLowerCase() + (k.getAttribute('role') === 'separator' ? ':sep' : ''))).toEqual(['section', 'div:sep', 'section'])
    expect(kids[2].textContent).toBe('More')
  })

  it('without further sections there is no trailing separator', () => {
    panel()
    expect(document.querySelector('[role="separator"]')).toBeNull()
  })

  it('names that are too long ellipsize instead of spilling; the count never shrinks', () => {
    const css = readFileSync(resolve(__dirname, 'FilterList.svelte'), 'utf8')
    expect(css).toMatch(/\.name\s*\{[^}]*min-width:\s*0[^}]*overflow:\s*hidden[^}]*text-overflow:\s*ellipsis[^}]*white-space:\s*nowrap/)
    expect(css).toMatch(/\.n\s*\{[^}]*flex-shrink:\s*0/)
    const panelCss = readFileSync(resolve(__dirname, 'GameFilterPanel.svelte'), 'utf8')
    // the panel keeps its width (the feed beside it gives way) and scrolls rather than spill
    expect(panelCss).toMatch(/flex-shrink:\s*0/)
    expect(panelCss).toMatch(/overflow-y:\s*auto/)
    // colour tokens only
    for (const src of [css, panelCss]) expect(src.match(/<style>([\s\S]*)<\/style>/)![1]).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i)
  })
})

describe('FilterList', () => {
  it('a section of its own: header, the "all" row, then rows; the kind names the list', () => {
    const onSelect = vi.fn()
    cmp = mount(FilterList, {
      target: document.body,
      props: { heading: 'People', kind: 'people', allLabel: 'Everyone', allCount: 6, rows: [{ id: 'U2', name: 'sam', count: 5 }], selected: 'U2', onSelect },
    })
    flushSync()
    const list = document.querySelector<HTMLElement>('.people')!
    const btns = [...list.querySelectorAll<HTMLButtonElement>('button')]
    expect(btns.map((b) => [b.querySelector('.name')?.textContent, b.querySelector('.n')?.textContent])).toEqual([['Everyone', '6'], ['sam', '5']])
    expect(btns.map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'true'])
    btns[0].click()
    expect(onSelect).toHaveBeenLastCalledWith(undefined)
  })
})
