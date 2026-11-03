// Badges (#49, docs/superpowers/specs/2026-11-03-badges-design.md): the catalogue (fetched once per
// account, again after an unlock), the unseen count behind [NEW] and the avatar dot, the unlock
// toasts, the logo egg, and the text the trophy case shows.
import { api } from './tauri'
import { app, loadProfile, onAppEvent } from './store.svelte'
import { setProfile } from './state'
import { playSound } from './voiceui'
import { openFull } from './profileCard.svelte'
import type { Event } from './protocol/Event'
import type { Badge } from './protocol/Badge'
import type { BadgeCatalogue } from './protocol/BadgeCatalogue'
import type { BadgeDef } from './protocol/BadgeDef'
import type { BadgeRarity } from './protocol/BadgeRarity'

/** Least to most rare: the showcase default and the trophy case sort by it. */
export const RARITIES: readonly BadgeRarity[] = ['common', 'uncommon', 'rare', 'epic', 'legendary', 'mythical']
export const SHOWCASE_MAX = 5
/** How long an unlock toast stays up. */
export const TOAST_MS = 6_000

export type BadgeToastItem = { id: number; name: string; icon: string; rarity: BadgeRarity; tier: number }

class BadgeStore {
  /** The catalogue as `owner` may see it (hidden badges are masked per viewer). */
  catalogue = $state.raw<BadgeCatalogue | null>(null)
  owner: string | null = null
  /** Unlocks since the last markSeen that your cached profile may not know of yet. */
  bumped = $state(0)
  toasts = $state<BadgeToastItem[]>([])
  #loading: Promise<void> | null = null
  #stale = false
  #nextToast = 1

  /** Your badges not yet seen in your trophy case: the [NEW] dot. */
  get unseen(): number {
    const me = app.state.me
    const own = me ? (app.state.profiles[me.id]?.badges.filter((b) => !b.seen).length ?? 0) : 0
    return Math.max(this.bumped, own)
  }

  /** Fetch the catalogue unless it's already here (and fresh, and yours); calls share one request. */
  load(): Promise<void> {
    const me = app.state.me?.id ?? null
    if (this.catalogue && !this.#stale && this.owner === me) return Promise.resolve()
    if (this.#loading) return this.#loading
    this.#stale = false
    const p = api
      .getBadgeCatalogue()
      .then((c) => {
        if (c && Array.isArray(c.badges)) {
          this.catalogue = c
          this.owner = me
        }
      })
      .catch(() => {})
      .finally(() => {
        if (this.#loading === p) this.#loading = null
      })
    this.#loading = p
    return p
  }

  /** The catalogue changed (an unlock moves the shares and the progress): refetch it if it's in use. */
  invalidate(): Promise<void> {
    this.#stale = true
    if (!this.catalogue) return Promise.resolve()
    this.#loading = null
    return this.load()
  }

  def(key: string): BadgeDef | undefined {
    return this.catalogue?.badges.find((d) => d.key === key)
  }

  /** You opened your trophy case: everything is seen, here and on the server. */
  markSeen() {
    this.bumped = 0
    const me = app.state.me
    const own = me ? app.state.profiles[me.id] : undefined
    if (own?.badges.some((b) => !b.seen)) app.state = setProfile(app.state, { ...own, badges: own.badges.map((b) => ({ ...b, seen: true })) })
    void api.markBadgesSeen().catch(() => {})
  }

  pushToast(t: Omit<BadgeToastItem, 'id'>) {
    const id = this.#nextToast++
    this.toasts = [...this.toasts, { ...t, id }]
    setTimeout(() => this.dismiss(id), TOAST_MS)
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id)
  }

  /** Signed out, or a test: nothing carries over. */
  reset() {
    this.catalogue = null
    this.owner = null
    this.bumped = 0
    this.toasts = []
    this.#loading = null
    this.#stale = false
  }
}

export const badgeStore = new BadgeStore()

/** A BadgeEarned (only ever sent to the earner): the toast, the sound, the [NEW] count, fresh data. */
export function handleBadgeEvent(e: Event) {
  if (e.t !== 'BadgeEarned') return
  const me = app.state.me
  if (!me || e.d.user_id !== me.id) return
  badgeStore.pushToast({ name: e.d.name, icon: e.d.icon, rarity: e.d.badge.rarity, tier: e.d.badge.tier })
  playSound('badge')
  badgeStore.bumped++
  void badgeStore.invalidate()
  loadProfile(me.id).catch(() => {})
}

/** Listen for unlocks; returns the unsubscribe function. */
export function startBadgeListening(): () => void {
  return onAppEvent(handleBadgeEvent)
}

/** The toast's click: your full profile, where the trophy case is. */
export function openOwnCase() {
  const me = app.state.me
  if (me) openFull(me.id, null)
}

export const rarityLabel = (r: BadgeRarity) => r[0].toUpperCase() + r.slice(1)
export const rarityVar = (r: BadgeRarity) => `var(--rarity-${r})`
export const rarityRank = (r: BadgeRarity) => RARITIES.indexOf(r)

/** "2 of 8 have this". */
export function shareText(holders: number, people: number): string {
  return `${holders} of ${people} ${holders === 1 ? 'has' : 'have'} this`
}

/** What each counter badge counts, for its progress line (the catalogue carries numbers only). */
const NOUNS: Record<string, string> = {
  the_yapper: 'messages',
  night_owl: 'nights',
  early_bird: 'days',
  ghost: 'drop-ins',
  the_doos: 'times',
  identity_crisis: 'renames',
  reaction_farmer: 'reactions',
  emoji_smith: 'uses',
  quotable: 'awards',
  john_quoter: 'quotes',
  personal_vendetta: 'quotes',
  clipper: 'clips',
  badge_collector: 'badges',
  how_did_we_get_here: 'hidden badges',
  perfectionist: 'maxed badges',
}

/** "412 / 500 messages" toward the next tier (the top one once all are reached); hours as "12 / 50 h". */
export function progressText(def: BadgeDef, value: number): string {
  const next = def.tiers.find((t) => t.threshold > value) ?? def.tiers[def.tiers.length - 1]
  const target = next?.threshold ?? 0
  if (def.unit === 'hours') return `${value} / ${target} h`
  const noun = NOUNS[def.key]
  return `${value.toLocaleString('en-US')} / ${target.toLocaleString('en-US')}${noun ? ` ${noun}` : ''}`
}

/** A badge the catalogue doesn't (yet) describe: its key in title case. */
export function fallbackName(key: string): string {
  return key.split(/[_\-\s]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(' ') || 'Badge'
}

/** Rarest first, then the older: the order the server picks the default showcase in. */
export function byRarity(a: Badge, b: Badge): number {
  return rarityRank(b.rarity) - rarityRank(a.rarity) || a.earned_at.localeCompare(b.earned_at)
}

/** The server's default showcase: the 5 rarest held, the older first among equals. */
export function defaultShowcase(badges: Badge[]): string[] {
  return [...badges].sort(byRarity).slice(0, SHOWCASE_MAX).map((b) => b.key)
}

/** I, II, III… for a tier. */
export function romanTier(n: number): string {
  const table: [number, string][] = [[10, 'X'], [9, 'IX'], [5, 'V'], [4, 'IV'], [1, 'I']]
  let out = ''
  for (const [v, s] of table) while (n >= v) (out += s), (n -= v)
  return out
}

// The logo egg: 7 clicks on the top-bar logo within 3 s, reported once a session.
const EGG_CLICKS = 7
const EGG_WINDOW_MS = 3_000
let eggClicks: number[] = []
let eggSent = false

export function logoClick(now = Date.now()) {
  if (eggSent) return
  eggClicks = [...eggClicks.filter((t) => now - t < EGG_WINDOW_MS), now]
  if (eggClicks.length < EGG_CLICKS) return
  eggSent = true
  eggClicks = []
  void api.reportEgg('logo').catch(() => {})
}

/** For tests: a new session. */
export function resetEgg() {
  eggClicks = []
  eggSent = false
}
