/** Whether `el` has focus the way a keyboard user gets it (`:focus-visible`); a mouse click on a chip or link does not.
 *  Where the selector is unsupported the answer is yes: showing a bar too often beats hiding it from the keyboard. */
export function keyboardFocused(el: EventTarget | null): boolean {
  try {
    return (el as Element).matches(':focus-visible')
  } catch {
    return true
  }
}

/** The `onfocusin`/`onfocusout` pair of an item with a hover action bar: the bar stays open while
 *  focus within the item is the keyboard's (the latest focus decides, so a mouse focus closes it). */
export function actionBarFocus(set: (open: boolean) => void) {
  return {
    onfocusin: (e: FocusEvent) => set(keyboardFocused(e.target)),
    onfocusout: (e: FocusEvent) => {
      if (!(e.currentTarget as Element).contains(e.relatedTarget as Node | null)) set(false)
    },
  }
}
