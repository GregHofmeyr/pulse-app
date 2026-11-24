// Your own stream's preview, hidden or shown for you (#603). Remembered on this device, like it always was: in the call
// grid hiding takes your whole stream tile away (the voice controls get a small button that brings it back); in a DM or
// group call's band only the video goes. Never seen by anyone else, and the stream itself keeps going.
import { loadHidePreview, saveHidePreview } from './prefs'

export const selfPreview = $state({ hidden: loadHidePreview() })

export function setSelfPreviewHidden(hidden: boolean): void {
  selfPreview.hidden = hidden
  saveHidePreview(hidden)
}

/** Tests: read the stored choice again. */
export function reloadSelfPreview(): void {
  selfPreview.hidden = loadHidePreview()
}
