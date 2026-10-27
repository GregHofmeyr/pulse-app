/** Whether `el` has focus the way a keyboard user gets it (`:focus-visible`); a mouse click on a chip or link does not.
 *  Where the selector is unsupported the answer is yes: showing a bar too often beats hiding it from the keyboard. */
export function keyboardFocused(el: EventTarget | null): boolean {
  try {
    return (el as Element).matches(':focus-visible')
  } catch {
    return true
  }
}
