/** What a dialog, popover or menu looks like (they take Esc before the view under them). */
const OVER = '[role="dialog"], [aria-modal="true"], [role="menu"], [role="listbox"], .emoji-popover'

/** Hidden by CSS (itself or an ancestor): `checkVisibility` where the engine has it, else the computed `display`. */
function visible(el: HTMLElement): boolean {
  if (el.checkVisibility) return el.checkVisibility()
  for (let n: HTMLElement | null = el; n; n = n.parentElement) if (getComputedStyle(n).display === 'none') return false
  return true
}

/** A dialog, popover or menu that's actually showing (one left open in a panel the app hid isn't). */
export function overlayShown(): boolean {
  return [...document.querySelectorAll<HTMLElement>(OVER)].some((el) => !el.closest('[hidden]') && visible(el))
}
