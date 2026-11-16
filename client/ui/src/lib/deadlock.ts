// Deadlock (#390): how ranks, matches and squad stats read. Pure.
import type { DeadlockRank } from './protocol/DeadlockRank'

/**
 * Where deadlock-api gives out its bot's Steam friend invite. The invites themselves rotate and are handed out on this
 * page of its official site, so Pulse links the page rather than a bot profile it can't vouch for.
 */
export const DEADLOCK_BOT_PAGE = 'https://deadlock-api.com/patron'
/** Shown next to the bot link: copycat "stats bots" are a known Steam scam. */
export const BOT_WARNING = 'The real bot never asks for logins, codes or trades — ignore copycats.'

/** "Oracle 3", "Obscurus". */
export function rankText(r: DeadlockRank): string {
  return r.tier === 0 || r.subrank === 0 ? r.name : `${r.name} ${r.subrank}`
}

/** "12 / 3 / 8". */
export const kdaText = (k: number, d: number, a: number): string => `${k} / ${d} / ${a}`

/** 1864 → "31:04", 3729 → "1:02:09". */
export function durationText(secs: number): string {
  const s = Math.max(0, Math.round(secs))
  const h = Math.floor(s / 3600)
  const m = Math.floor(s / 60) % 60
  const ss = String(s % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`
}

/** 112 → "+112", -90 → "−90". */
export const deltaText = (n: number): string => (n > 0 ? `+${n}` : n < 0 ? `−${-n}` : '±0')

/** "6 of 10 won · 60 %". */
export function winRateText(wins: number, games: number): string {
  if (games <= 0) return ''
  return `${wins} of ${games} won · ${Math.round((wins / games) * 100)} %`
}

/** A mover's subranks: 2 → "▲ 2", -3 → "▼ 3". */
export const stepsText = (steps: number): string => (steps >= 0 ? `▲ ${steps}` : `▼ ${-steps}`)
