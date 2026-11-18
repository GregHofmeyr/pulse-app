// Where a menu opened at the cursor goes (#181): at the cursor, kept 8px inside the window, and the corner its
// pop-in grows from. That is its top left, unless an edge pushed it back over the cursor: then the corner nearest
// the cursor, so it still seems to come out of the pointer.

/** The room kept between a menu and the window's edges. */
const EDGE = 8

export type MenuPlace = { left: number; top: number; origin: string }

export function placeMenu(x: number, y: number, width: number, height: number, win = { width: window.innerWidth, height: window.innerHeight }): MenuPlace {
  const left = Math.max(EDGE, Math.min(x, win.width - width - EDGE))
  const top = Math.max(EDGE, Math.min(y, win.height - height - EDGE))
  const h = left < x && x - left > width / 2 ? 'right' : 'left'
  const v = top < y && y - top > height / 2 ? 'bottom' : 'top'
  return { left, top, origin: `${v} ${h}` }
}

/** The room a flyout takes beside its menu: the 4px gap and its 200px width. */
const FLYOUT_ROOM = 4 + 200

export type FlyoutPlace = { side: 'left' | 'right'; shift: number }

/** Which side of its menu a flyout opens on (#131), and how far it's pulled back over the menu (#134): the left
 *  when it fits there, else the right; with room on neither side, the roomier one, `shift` px back toward the menu
 *  so it stays 8px inside the window. */
export function placeFlyout(menuLeft: number, menuWidth: number, winWidth = window.innerWidth): FlyoutPlace {
  const leftRoom = menuLeft - EDGE
  const rightRoom = winWidth - EDGE - (menuLeft + menuWidth)
  if (leftRoom >= FLYOUT_ROOM) return { side: 'left', shift: 0 }
  if (rightRoom >= FLYOUT_ROOM) return { side: 'right', shift: 0 }
  return leftRoom > rightRoom ? { side: 'left', shift: FLYOUT_ROOM - leftRoom } : { side: 'right', shift: FLYOUT_ROOM - rightRoom }
}

/** A button's box in the window. */
export type Anchor = { left: number; right: number; top: number; bottom: number }

/** Where a popover hung off a button goes (#454): `top` when below, `bottom` (from the window's bottom) when above. */
export type PopoverPlace = { left: number; below: boolean; top?: number; bottom?: number; maxHeight: number }

/** A popover hung off a button: below it when it fits, else above when it fits there, else on the roomier side.
 *  Lined up with the button's start (left) or end (right) edge, kept EDGE px inside the window across, and never
 *  taller than the room on its side (`maxHeight`: give the popover that and `overflow-y: auto`, so a long one
 *  scrolls instead of running off the screen). Above, it's pinned by its bottom, so its height needn't be known. */
export function placePopover(a: Anchor, size: { width: number; height: number }, win = { width: window.innerWidth, height: window.innerHeight }, align: 'start' | 'end' = 'start', gap = 6): PopoverPlace {
  const roomBelow = Math.max(0, win.height - a.bottom - gap - EDGE)
  const roomAbove = Math.max(0, a.top - gap - EDGE)
  const below = size.height <= roomBelow || (size.height > roomAbove && roomBelow >= roomAbove)
  const want = align === 'end' ? a.right - size.width : a.left
  const left = Math.max(EDGE, Math.min(want, win.width - size.width - EDGE))
  return below
    ? { left, below, top: a.bottom + gap, maxHeight: roomBelow }
    : { left, below, bottom: win.height - a.top + gap, maxHeight: roomAbove }
}
