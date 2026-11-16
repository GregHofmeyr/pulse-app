// Steam achievement cards (#379): how a card reads. Pure.
import { isMediaHash, mediaUrl } from './emoji'
import type { EmbedThumb } from './protocol/EmbedThumb'

/** How rare an achievement is, by the share of players who have it, in the badge rarity colours. */
export function rarityOf(percent: number | null | undefined): 'common' | 'uncommon' | 'rare' | 'epic' | 'legendary' | null {
  if (percent == null) return null
  if (percent < 5) return 'legendary'
  if (percent < 10) return 'epic'
  if (percent < 25) return 'rare'
  if (percent < 50) return 'uncommon'
  return 'common'
}

/** "12.3 % of players", "<0.1 % of players". */
export function percentText(percent: number): string {
  return percent < 0.1 ? '<0.1 % of players' : `${Math.round(percent * 10) / 10} % of players`
}

/** "2 / 3 · 67 %". */
export function progressText(unlocked: number, total: number): string {
  if (total <= 0) return ''
  return `${unlocked} / ${total} · ${Math.floor((Math.min(unlocked, total) / total) * 100)} %`
}

/** The share done, 0 to 1. */
export const progressShare = (unlocked: number, total: number): number => (total > 0 ? Math.min(1, Math.max(0, unlocked / total)) : 0)

/** A thumb's media URL, or null. */
export const thumbUrl = (t: EmbedThumb | null | undefined): string | null => (t && isMediaHash(t.hash) ? mediaUrl(t.hash) : null)

/** "62 % completion · 3 perfect games", once there's a rate. */
export function completionLine(rate: number | null | undefined, perfect: number | null | undefined): string | null {
  if (rate == null) return null
  const p = perfect ?? 0
  const games = p > 0 ? ` · ${p} perfect game${p === 1 ? '' : 's'}` : ''
  return `${formatRate(rate)} completion${games}`
}

/** Steam hours (#383): 45 → "45 min", 1290 → "21.5 h". */
export function hoursText(minutes: number): string {
  const m = Math.max(0, Math.round(minutes))
  return m < 60 ? `${m} min` : `${Math.round(m / 6) / 10} h`
}

/** 62.5 → "62.5 %", 100 → "100 %". */
export const formatRate = (rate: number): string => `${Math.round(rate * 10) / 10} %`
