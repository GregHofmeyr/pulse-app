// @vitest-environment jsdom
// A sports card (#385): match day with kick-off in local time and no score, live with the accent corner and the
// minute, full time with the scorers per side and the winner, and who follows either team.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import SportsCard from './SportsCard.svelte'
import type { SportsCard as Card } from '../lib/protocol/SportsCard'

const names: Record<string, string> = { U1: 'alex', U2: 'sam' }
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const base: Card = {
  league: 'psl', fixture_id: 1, phase: 'upcoming', kickoff: '2026-11-21T17:30:00Z', status: 'NS', minute: null,
  home: { team_id: 10, name: 'Coastal City', score: null }, away: { team_id: 20, name: 'Highveld United', score: null },
  scorers: [], followers: ['U2'],
}

function show(over: Partial<Card>) {
  cmp = mount(SportsCard, { target: document.body, props: { card: { ...base, ...over }, nameOf: (id: string) => names[id] ?? '?', time: '09:00', messageId: 'M1' } })
  flushSync()
  return document.querySelector<HTMLElement>('.sports')!
}

describe('SportsCard', () => {
  it('match day: both teams, kick-off in local time, no score, who follows', () => {
    const el = show({})
    const local = new Date(base.kickoff).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    expect(el.querySelector('[data-corner]')!.textContent).toContain(local)
    expect(el.querySelector('[data-corner]')!.classList.contains('live')).toBe(false)
    expect(el.querySelectorAll('.score')).toHaveLength(0)
    expect(el.querySelector('.head')!.textContent).toContain('Premiership (PSL)')
    expect(el.querySelector('.followers')!.textContent).toBe('Followed by sam')
  })

  it('live: the score and the minute on the accent corner', () => {
    const el = show({ phase: 'live', status: '2H', minute: 63, home: { ...base.home, score: 1 }, away: { ...base.away, score: 0 }, followers: ['U1', 'U2'] })
    const corner = el.querySelector('[data-corner]')!
    expect(corner.textContent).toBe("63'")
    expect(corner.classList.contains('live')).toBe(true)
    expect([...el.querySelectorAll('.score')].map((s) => s.textContent)).toEqual(['1', '0'])
    expect(el.querySelector('.followers')!.textContent).toBe('Followed by alex and sam')
  })

  it('full time: scorers per side and the winner marked', () => {
    const el = show({
      phase: 'final', status: 'FT', home: { ...base.home, score: 2 }, away: { ...base.away, score: 1 },
      scorers: [
        { home: true, name: 'A. Striker', minute: 41, note: 'pen' },
        { home: false, name: 'E. Nine', minute: 61, note: null },
        { home: true, name: 'F. Late', minute: 90, note: null },
      ],
    })
    expect(el.querySelector('[data-corner]')!.textContent).toBe('Full time')
    expect(el.querySelector('[data-side=home]')!.classList.contains('won')).toBe(true)
    const [home, away] = el.querySelectorAll('.scorers span')
    expect(home.textContent).toBe("A. Striker 41' (pen), F. Late 90'")
    expect(away.textContent).toBe("E. Nine 61'")
  })

  it('rugby and a called-off match', () => {
    const el = show({ league: 'urc', phase: 'off', status: 'PST', followers: [] })
    expect(el.querySelector('.head')!.textContent).toContain('🏉')
    expect(el.querySelector('[data-corner]')!.textContent).toBe('Postponed')
    expect(el.querySelector('.followers')).toBeNull()
  })
})
