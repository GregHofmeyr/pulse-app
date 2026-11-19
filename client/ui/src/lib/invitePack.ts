// The invite card pack (#511), the pure part: when a card arrives sealed, how a drag across its top tears it, and
// remembering on this device that it was opened. The look and the motion are ServerInviteCard.svelte's.
//
// A pack is only for someone the invite is for: its sender sees their card already open, and a dead invite (expired,
// used, revoked, someone else's, banned) is the greyed card with why. With reduced motion it still arrives sealed, held
// still, and a click, Enter or a short drag opens it at once, with no flight, burst or rise (#538). Once opened it
// stays open on this device (per invite code); it never opens by itself.

export type PackStage = 'sealed' | 'ripping' | 'open'

/** How much of the width a tear must cover before it rips. */
export const TEAR_AT = 0.8
/** With reduced motion (#538), a short drag is enough: it opens at once instead of tearing. */
export const SHORT_DRAG_AT = 0.15

/** How far a drag must go to open the pack. */
export const ripAt = (reducedMotion: boolean): number => (reducedMotion ? SHORT_DRAG_AT : TEAR_AT)
/** From the rip to the open card: the strip flies off and the card rises out. */
export const RIP_MS = 600
/** Only a press this near the top of the pack starts a tear (px from its top). */
export const BAND_PX = 110

/** The stage a card starts in. `valid`: the invite still lets you in and you aren't in yet. Reduced motion doesn't
 *  change it (#538): the pack arrives sealed either way, only how it opens differs. */
export function startStage(o: { mine: boolean; valid: boolean; opened: boolean }): PackStage {
  return o.valid && !o.mine && !o.opened ? 'sealed' : 'open'
}

/** Opening it: through the ripping stage (the strip flies, the card rises), or with reduced motion straight to open. */
export const openingStage = (reducedMotion: boolean): PackStage => (reducedMotion ? 'open' : 'ripping')

/** A tear in progress: whether the pointer is held, the stretch it has covered (0–1 of the width, either way), and
 *  where it is now. */
export type Tear = { dragging: boolean; lo: number; hi: number; cursor: number }
export const NO_TEAR: Tear = { dragging: false, lo: 0, hi: 0, cursor: 0 }

/** How far a tear has gone (0–1). */
export const progressOf = (t: Tear): number => (t.dragging ? t.hi - t.lo : 0)

/** A press at `x` (0–1 across) and `y` (px from the top): a tear starts there, or null (not on the top band). */
export function press(x: number, y: number): Tear | null {
  if (y > BAND_PX) return null
  const at = clamp(x)
  return { dragging: true, lo: at, hi: at, cursor: at }
}

/** The pointer moved to `x`: the tear stretches to cover it. `rip`: it went far enough to open (`openAt`: [`ripAt`]). */
export function stretch(t: Tear, x: number, openAt: number = TEAR_AT): { tear: Tear; rip: boolean } {
  if (!t.dragging) return { tear: t, rip: false }
  const at = clamp(x)
  const tear = { dragging: true, lo: Math.min(t.lo, at), hi: Math.max(t.hi, at), cursor: at }
  return { tear, rip: progressOf(tear) >= openAt }
}

const clamp = (x: number) => Math.min(1, Math.max(0, x))

const KEY = (code: string) => `pulse.invitePack.${code}`

/** Whether this invite's pack was opened on this device (storage that can't be read: not yet). */
export function wasOpened(code: string): boolean {
  try {
    return localStorage.getItem(KEY(code)) === '1'
  } catch {
    return false
  }
}

/** Remember on this device that this invite's pack is open (storage that can't be written: it just isn't kept). */
export function markOpened(code: string): void {
  try {
    localStorage.setItem(KEY(code), '1')
  } catch {
    /* kept for this session only */
  }
}

/** Whether this device asks for less motion. */
export function prefersReducedMotion(): boolean {
  try {
    return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return false
  }
}
