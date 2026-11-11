// How the stage shows a Spotify share for you (#234), remembered on this device: the strip above the tiles, the
// floating pill, or hidden (until the header's Spotify button). A new share shows the strip again, unless you'd hidden
// the one before it. The corner card is the pill grown open (#288): a moment's view, never kept.

export type StageMode = 'strip' | 'pill' | 'hidden'

const KEY = 'pulse.stageShare'
const MODES: StageMode[] = ['strip', 'pill', 'hidden']

type Saved = { mode: StageMode; shareId: string | null }

function load(): Saved {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    if (v && typeof v === 'object') {
      const o = v as Record<string, unknown>
      // 'corner' was kept before #288: it's the pill now, which opens into the card
      const raw = o.mode === 'corner' ? 'pill' : o.mode
      const mode = MODES.includes(raw as StageMode) ? (raw as StageMode) : 'strip'
      return { mode, shareId: typeof o.shareId === 'string' ? o.shareId : null }
    }
  } catch {
    /* ignore */
  }
  return { mode: 'strip', shareId: null }
}

export const stageView = $state(load())

/** How `shareId` shows for you now. */
export function modeFor(shareId: string): StageMode {
  if (stageView.shareId === shareId) return stageView.mode
  return stageView.mode === 'hidden' ? 'hidden' : 'strip'
}

/** Show `shareId` as `mode` from now on (on this device). */
export function setMode(shareId: string, mode: StageMode): void {
  stageView.mode = mode
  stageView.shareId = shareId
  try {
    localStorage.setItem(KEY, JSON.stringify({ mode, shareId }))
  } catch {
    /* ignore */
  }
}

/** Read back from storage (tests: as after a restart). */
export function reloadStageView(): void {
  Object.assign(stageView, load())
}
