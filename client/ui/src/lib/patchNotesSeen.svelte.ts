// Which release's patch notes were last opened on this device (#207): the title bar icon carries a dot until the
// current notes' id is the one stored here.
const KEY = 'pulse.patchNotesSeen'

function saved(): string | null {
  try {
    return localStorage.getItem(KEY)
  } catch {
    return null
  }
}

export const patchNotesSeen = $state<{ id: string | null }>({ id: saved() })

export function markPatchNotesSeen(id: string): void {
  patchNotesSeen.id = id
  try {
    localStorage.setItem(KEY, id)
  } catch {
    /* ignore */
  }
}

/** Re-read from storage (tests, as if the app had restarted). */
export function reloadPatchNotesSeen(): void {
  patchNotesSeen.id = saved()
}
