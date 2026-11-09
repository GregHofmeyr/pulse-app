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
