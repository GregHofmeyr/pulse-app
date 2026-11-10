// Strong (low delay)'s model (#77): the core downloads it from the signed-in server on first use and checks its hash;
// this holds where it is for Voice settings. Picking the level starts the download; Strong runs until it's ready.
import { listen } from '@tauri-apps/api/event'
import { api, errorText, type ModelStatus } from './tauri'

export const lowDelayModel = $state({ status: { state: 'missing' } as ModelStatus })

/** Ask the core where it is now. */
export async function refreshModel(): Promise<void> {
  try {
    lowDelayModel.status = await api.modelStatus()
  } catch {
    /* the line just stays as it was */
  }
}

/** Download it, unless it's there or on its way already (asked fresh: the page's copy may be stale). */
export async function downloadModel(): Promise<void> {
  await refreshModel()
  const st = lowDelayModel.status.state
  if (st === 'ready' || st === 'downloading') return
  lowDelayModel.status = { state: 'downloading', downloaded: 0, total: null }
  try {
    lowDelayModel.status = await api.modelDownload()
  } catch (e) {
    lowDelayModel.status = { state: 'failed', error: errorText(e) }
  }
}

/** The core's progress events while the page is open. */
export async function listenModel(): Promise<() => void> {
  return listen<ModelStatus>('pulse://model', (e) => {
    lowDelayModel.status = e.payload
  })
}
