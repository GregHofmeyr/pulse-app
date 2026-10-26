// Whether the Pulse window is focused: the one source for message sounds, read marking and
// animated emoji. In Tauri the OS window's focus event decides once subscribed (a webview's DOM
// blur can also mean keyboard focus left the webview while the window stays active). DOM
// focus/blur and document.hasFocus() are the fallback, and the value until the first Tauri event.
import { getCurrentWindow } from '@tauri-apps/api/window'

export const windowFocus = $state({ focused: true })

let watching = false
let native = false

/** Starts following window focus, once for the app's lifetime. */
export function watchWindowFocus(): void {
  if (watching) return
  watching = true
  windowFocus.focused = document.hasFocus()
  window.addEventListener('focus', () => {
    if (!native) windowFocus.focused = true
  })
  window.addEventListener('blur', () => {
    if (!native) windowFocus.focused = false
  })
  if (!('__TAURI_INTERNALS__' in window)) return
  let heard = false
  try {
    const win = getCurrentWindow()
    win
      .onFocusChanged(({ payload }) => {
        heard = true
        windowFocus.focused = payload
      })
      .then(
        () => {
          native = true
          // hasFocus() can read false at startup in a focused window, and no focus event follows
          // until focus changes: ask the window once. A focus event that came first is newer.
          win.isFocused().then(
            (f) => {
              if (!heard) windowFocus.focused = f
            },
            () => {},
          )
        },
        () => {}, // DOM events only
      )
  } catch {
    /* DOM events only */
  }
}
