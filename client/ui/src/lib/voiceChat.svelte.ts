// A voice channel's chat panel (#226): open or closed on this device (its width is a column, lib/columns.svelte.ts),
// and the focus a click asks for. The panel shows beside the voice stage, in the right column, while it's open.

const KEY = 'pulse.voiceChat'

/** The stage's chat toggle: closing the panel from inside hands the focus back to it. */
export const CHAT_TOGGLE_ID = 'voice-chat-toggle'

/** A stream in full view (#63) shows the same chat as a strip beside the video, inside the full view (so it stays in
 *  full screen): its id, and its toggle's in the full view's bar. Open or closed is the panel's own `open`. */
export const STRIP_ID = 'stream-chat'
export const STRIP_TOGGLE_ID = 'stream-chat-toggle'

function saved(): boolean {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return !!v && typeof v === 'object' && (v as Record<string, unknown>).open === true
  } catch {
    return false
  }
}

/** `focusWanted` / `focusDone`: a click that opens the chat asks for its message box (#202); the panel takes the
 *  focus once for each ask, whether it was just mounted or already open. */
export const voiceChat = $state({ open: saved(), focusWanted: 0, focusDone: 0 })

function store() {
  try {
    localStorage.setItem(KEY, JSON.stringify({ open: voiceChat.open }))
  } catch {
    /* ignore */
  }
}

/** Open or close the panel. `focus`: opened by a click, so its message box takes the focus. */
export function setVoiceChat(open: boolean, focus = false): void {
  if (voiceChat.open !== open) {
    voiceChat.open = open
    store()
  }
  if (open && focus) voiceChat.focusWanted++
}

/** The stage's toggle. */
export function toggleVoiceChat(focus = false): void {
  setVoiceChat(!voiceChat.open, focus)
}

/** Whether a focus is waiting; taking it marks it done. */
export function takeChatFocus(): boolean {
  if (voiceChat.focusWanted <= voiceChat.focusDone) return false
  voiceChat.focusDone = voiceChat.focusWanted
  return true
}

/** Read back from storage (tests: as after a restart). */
export function reloadVoiceChat(): void {
  voiceChat.open = saved()
  voiceChat.focusDone = voiceChat.focusWanted
}
