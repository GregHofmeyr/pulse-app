// Sports score pings (#385): how leagues, matches and cards read. Pure.
import type { SportsCard } from './protocol/SportsCard'
import type { SportsLeague } from './protocol/SportsLeague'
import type { SportsScorer } from './protocol/SportsScorer'
import type { SportsTeam } from './protocol/SportsTeam'

export const LEAGUE_NAMES: Record<SportsLeague, string> = {
  psl: 'Premiership (PSL)',
  urc: 'United Rugby Championship',
  epl: 'Premier League',
}

export const leagueName = (l: SportsLeague): string => LEAGUE_NAMES[l] ?? l
export const isRugby = (l: SportsLeague): boolean => l === 'urc'
export const ballOf = (l: SportsLeague): string => (isRugby(l) ? '🏉' : '⚽')

/** Kick-off in your own time: "19:30", or "Sat 19:30" when it isn't today. */
export function kickoffText(iso: string, now: number): string {
  const t = new Date(iso)
  const time = t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  return t.toDateString() === new Date(now).toDateString() ? time : `${t.toLocaleDateString([], { weekday: 'short' })} ${time}`
}

/** What the card's corner says: "Kick-off 19:30", "63'", "Half time", "Full time", "Postponed". */
export function phaseText(c: SportsCard, now: number): string {
  switch (c.phase) {
    case 'upcoming':
      return `Kick-off ${kickoffText(c.kickoff, now)}`
    case 'live':
      if (c.status === 'HT') return 'Half time'
      return c.minute != null ? `${c.minute}'` : 'Live'
    case 'final':
      return c.status === 'AET' || c.status === 'AOT' ? 'Full time (extra time)' : c.status === 'PEN' ? 'Full time (penalties)' : 'Full time'
    case 'off':
      return c.status === 'CANC' ? 'Cancelled' : c.status === 'ABD' ? 'Abandoned' : 'Postponed'
  }
}

/** "A. Striker 41' (pen)". */
export function scorerText(s: SportsScorer): string {
  const minute = s.minute != null ? ` ${s.minute}'` : ''
  const note = s.note ? ` (${s.note})` : ''
  return `${s.name}${minute}${note}`
}

/** A league's teams whose names contain every word typed (any case, accents ignored). */
export function matchTeams(teams: SportsTeam[], query: string): SportsTeam[] {
  const fold = (s: string) => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
  const words = fold(query).split(/\s+/).filter(Boolean)
  return teams.filter((t) => words.every((w) => fold(t.name).includes(w)))
}

export const sameTeam = (a: Pick<SportsTeam, 'league' | 'team_id'>, b: Pick<SportsTeam, 'league' | 'team_id'>): boolean =>
  a.league === b.league && a.team_id === b.team_id

/** "alex", "alex and sam", "alex, sam and kim". */
export function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? ''
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}
