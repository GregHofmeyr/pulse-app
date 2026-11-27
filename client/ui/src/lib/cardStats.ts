// Card stats (#727): the lines of the stats panel on your card that you can show, hide and reorder. The keys and
// their order are the server's (`CARD_STATS` in server/src/profiles/mod.rs); a literal copy is checked against it in
// cardStats.test.ts.
export const CARD_STATS = ['member_since', 'voice', 'steam_completion', 'steam_recent'] as const
export type CardStat = (typeof CARD_STATS)[number]

export const CARD_STAT_LABELS: Record<CardStat, string> = {
  member_since: 'Member since',
  voice: 'Voice',
  steam_completion: 'Steam completion',
  steam_recent: 'Steam, 2 weeks',
}

/** The shown stats, in order: no choice (null) is all of them in the server's order. */
export const shownStats = (cardStats: string[] | null): string[] => (cardStats ?? [...CARD_STATS])

/** Whether a stat line shows: the owner's `shown` keys when the server sends them, else every line. */
export const statShown = (shown: readonly string[] | null | undefined, key: string): boolean => !shown || shown.includes(key)
