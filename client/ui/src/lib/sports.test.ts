// Sports score pings (#385): kick-off in your own time, the card's corner, scorers, the team search.
import { describe, expect, it } from 'vitest'
import { ballOf, joinNames, kickoffText, leagueName, logoUrl, matchTeams, phaseText, scorerText } from './sports'
import type { SportsCard } from './protocol/SportsCard'

const card = (over: Partial<SportsCard>): SportsCard => ({
  league: 'psl', fixture_id: 1, phase: 'upcoming', kickoff: '2026-11-21T17:30:00Z', status: 'NS', minute: null,
  home: { team_id: 10, name: 'Coastal City', score: null }, away: { team_id: 20, name: 'Highveld United', score: null },
  scorers: [], followers: [], no_updates: false, ...over,
})

describe('sports', () => {
  it('a card out of live data says so in its corner; a logo is a media hash or nothing (#406)', () => {
    expect(phaseText(card({ phase: 'live', minute: 70, no_updates: true }), Date.now())).toBe('No live updates')
    expect(logoUrl({ hash: 'a'.repeat(64), width: 96, height: 96 })).toContain('a'.repeat(64))
    expect(logoUrl({ hash: 'https://evil.test/x.png', width: 1, height: 1 })).toBeNull()
    expect(logoUrl(undefined)).toBeNull()
  })

  it('names leagues and their ball', () => {
    expect(leagueName('urc')).toBe('United Rugby Championship')
    expect(ballOf('urc')).toBe('🏉')
    expect(ballOf('epl')).toBe('⚽')
  })

  it('shows kick-off in local time, with the day when it is not today', () => {
    const iso = '2026-11-21T17:30:00Z'
    const local = new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    expect(kickoffText(iso, Date.parse(iso) - 3600_000)).toBe(local)
    expect(kickoffText(iso, Date.parse(iso) - 3 * 86_400_000)).toMatch(new RegExp(`^\\S+ ${local}$`))
  })

  it("reads the card's corner for each phase", () => {
    const now = Date.parse('2026-11-21T12:00:00Z')
    expect(phaseText(card({}), now)).toMatch(/^Kick-off /)
    expect(phaseText(card({ phase: 'live', status: '2H', minute: 63 }), now)).toBe("63'")
    expect(phaseText(card({ phase: 'live', status: 'HT', minute: 45 }), now)).toBe('Half time')
    expect(phaseText(card({ phase: 'live', status: '1H' }), now)).toBe('Live')
    expect(phaseText(card({ phase: 'final', status: 'FT' }), now)).toBe('Full time')
    expect(phaseText(card({ phase: 'final', status: 'PEN' }), now)).toBe('Full time (penalties)')
    expect(phaseText(card({ phase: 'off', status: 'PST' }), now)).toBe('Postponed')
    expect(phaseText(card({ phase: 'off', status: 'CANC' }), now)).toBe('Cancelled')
  })

  it('writes scorers with their minute and note', () => {
    expect(scorerText({ home: true, name: 'A. Striker', minute: 41, note: 'pen' })).toBe("A. Striker 41' (pen)")
    expect(scorerText({ home: false, name: 'E. Nine', minute: null, note: null })).toBe('E. Nine')
  })

  it('finds teams by every word typed, any case, accents ignored', () => {
    const teams = [
      { league: 'psl' as const, team_id: 1, name: 'Coastal City' },
      { league: 'psl' as const, team_id: 2, name: 'Highveld United' },
      { league: 'psl' as const, team_id: 3, name: 'Équipe Côte' },
    ]
    expect(matchTeams(teams, 'city').map((t) => t.team_id)).toEqual([1])
    expect(matchTeams(teams, 'high  UNI').map((t) => t.team_id)).toEqual([2])
    expect(matchTeams(teams, 'cote').map((t) => t.team_id)).toEqual([3])
    expect(matchTeams(teams, '')).toHaveLength(3)
  })

  it('joins follower names', () => {
    expect(joinNames([])).toBe('')
    expect(joinNames(['alex'])).toBe('alex')
    expect(joinNames(['alex', 'sam', 'kim'])).toBe('alex, sam and kim')
  })
})
