// A public spin's card stays in the chat until you've seen it (#271): Pulse focused, that chat on screen and the card
// in view, at or after landing. Then it stays CARD_SEEN_MS more and folds into its 🎡 line; ✕ folds it at once; it
// folds anyway CARD_MAX_MS after the spin. Per device: what's been seen or dismissed is kept in localStorage by
// message id (so a reload doesn't bring a card back), and spins older than the hour are forgotten.

/** After you've seen a card, it stays this long. */
export const CARD_SEEN_MS = 20_000
/** A card folds this long after its spin, seen or not. */
export const CARD_MAX_MS = 60 * 60_000
export const WHEEL_CARDS_KEY = 'pulse.wheelCards'

/** `at`: when it was spun (ms); `seen`: when you first saw it landed; `done`: dismissed. */
type Rec = { at: number; seen?: number; done?: true }

let cards: Record<string, Rec> | null = null

const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined)

function load(): Record<string, Rec> {
  if (cards) return cards
  cards = {}
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(WHEEL_CARDS_KEY) ?? '{}')
    if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
      for (const [id, v] of Object.entries(raw as Record<string, unknown>)) {
        if (typeof v !== 'object' || v === null) continue
        const r = v as Record<string, unknown>
        const at = num(r.at)
        if (at === undefined) continue
        const seen = num(r.seen)
        cards[id] = { at, ...(seen !== undefined ? { seen } : {}), ...(r.done === true ? { done: true as const } : {}) }
      }
    }
  } catch {
    // unreadable: nothing remembered
  }
  return cards
}

function save(now: number) {
  const all = load()
  for (const [id, r] of Object.entries(all)) if (now - r.at >= CARD_MAX_MS) delete all[id]
  try {
    localStorage.setItem(WHEEL_CARDS_KEY, JSON.stringify(all))
  } catch {
    // storage unavailable: it lasts until the app closes
  }
}

/** Whether the card of the spin `id` (spun at `spunAt`) shows at `now`, when you first saw it, and when it folds. */
export function cardState(id: string, spunAt: number, now = Date.now()): { show: boolean; seen: number | null; foldAt: number } {
  const r = load()[id]
  const seen = r?.seen ?? null
  const foldAt = Math.min(spunAt + CARD_MAX_MS, seen === null ? Infinity : seen + CARD_SEEN_MS)
  return { show: !r?.done && now < foldAt, seen, foldAt }
}

/** You've seen the card (landed): it folds CARD_SEEN_MS from the first time. */
export function markCardSeen(id: string, spunAt: number, now = Date.now()): void {
  const all = load()
  if (all[id]?.seen !== undefined) return
  all[id] = { ...all[id], at: spunAt, seen: now }
  save(now)
}

/** ✕: folded for you, now. */
export function dismissCard(id: string, spunAt: number, now = Date.now()): void {
  const all = load()
  all[id] = { ...all[id], at: spunAt, done: true }
  save(now)
}

/** For tests (and the app starting): read localStorage afresh next time. */
export function resetWheelCards(): void {
  cards = null
}
