// @vitest-environment jsdom
// A Deadlock card (#390): a match with a row per friend (hero art, K/D/A, win or loss, rank and its change), the weekly
// rank movers, and a patch (title link, summary, hero chips). Drawn from a system message by MessageItem.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import DeadlockCard from './DeadlockCard.svelte'
import type { DeadlockCard as Card } from '../lib/protocol/DeadlockCard'

const HASH = 'cd'.repeat(32)
const thumb = { hash: HASH, width: 64, height: 64 }
const rank = (tier: number, subrank: number, name: string) => ({ badge: tier * 10 + subrank, tier, subrank, name, image: thumb })
const names: Record<string, string> = { U1: 'alex', U2: 'sam' }
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function show(card: Card) {
  cmp = mount(DeadlockCard, { target: document.body, props: { card, nameOf: (id: string) => names[id] ?? '?', time: '21:00', messageId: 'M1' } })
  flushSync()
  return document.querySelector<HTMLElement>('.dl')!
}

describe('DeadlockCard', () => {
  it('a match: one row per friend, win or loss, rank and its change', () => {
    const el = show({
      kind: 'match',
      match_id: 50000002,
      duration_s: 1864,
      players: [
        { user_id: 'U1', hero_id: 1, hero: 'Infernus', hero_art: thumb, kills: 12, deaths: 3, assists: 8, won: true, rank_delta: 112, rank: rank(8, 3, 'Oracle') },
        { user_id: 'U2', hero_id: 13, hero: 'Haze', hero_art: null, kills: 4, deaths: 7, assists: 10, won: false, rank_delta: null, rank: null },
      ],
    })
    expect(el.querySelector('.head')!.textContent).toContain('Deadlock match · 31:04')
    const rows = el.querySelectorAll('li')
    expect(rows).toHaveLength(2)
    expect(rows[0].textContent).toContain('alex as Infernus')
    expect(rows[0].querySelector('.kda')!.textContent).toBe('12 / 3 / 8')
    expect(rows[0].dataset.result).toBe('win')
    expect(rows[0].querySelector('.rank')!.textContent).toContain('Oracle 3 +112')
    expect(rows[0].querySelector('img.hero')!.getAttribute('src')).toContain(HASH)
    expect(rows[1].dataset.result).toBe('loss')
    expect(rows[1].querySelector('.result')!.textContent).toBe('Loss')
    expect(rows[1].querySelector('.hero.none')).toBeTruthy()
    expect(rows[1].querySelector('.rank')).toBeNull()
  })

  it('the rank movers, up and down', () => {
    const el = show({
      kind: 'rank_movers',
      movers: [
        { user_id: 'U1', from: rank(8, 3, 'Oracle'), to: rank(8, 5, 'Oracle'), steps: 2 },
        { user_id: 'U2', from: rank(7, 5, 'Emissary'), to: rank(7, 2, 'Emissary'), steps: -3 },
      ],
    })
    const rows = el.querySelectorAll('li')
    expect(rows[0].textContent).toContain('alex Oracle 3 → Oracle 5')
    expect(rows[0].querySelector('.steps')!.textContent).toBe('▲ 2')
    expect(rows[1].querySelector('.steps')!.textContent).toBe('▼ 3')
  })

  it('a patch: the link opens outside, the summary and the heroes it names', () => {
    const el = show({ kind: 'patch', title: 'Gameplay Update', link: 'https://news.example/1', published_at: '2026-11-05T19:58:14Z', summary: 'Haze: more smoke.', heroes: [{ name: 'Haze', icon: thumb }] })
    const a = el.querySelector<HTMLAnchorElement>('a.title')!
    expect(a.textContent).toBe('Gameplay Update')
    expect(a.getAttribute('href')).toBe('https://news.example/1')
    expect(a.getAttribute('rel')).toContain('noopener')
    expect(el.querySelector('.summary')!.textContent).toBe('Haze: more smoke.')
    expect(el.querySelector('.hero-chip')!.textContent).toBe('Haze')
  })
})
