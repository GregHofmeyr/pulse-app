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
