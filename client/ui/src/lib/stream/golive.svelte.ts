// Go Live's picker state, shared by the call controls and the participant menu: the dialog itself renders in the shell.
import { StreamRoom, type Picked } from './room'
import { isWindows } from './platform'
import { errorText } from '../tauri'

export const goLiveUi = $state({
  /** The captured source the dialog is configuring; the dialog owns the track until it hands it off. */
  picked: null as Picked | null,
  /** The channel it goes live in. */
  channelId: null as string | null,
  /** A picker failure, for the call view of that channel. */
  error: '',
})

/** Asks for a screen/window to share; the dialog opens once one is picked. */
export async function pickSource(channelId: string): Promise<void> {
  if (!isWindows()) return
  goLiveUi.error = ''
  try {
    const picked = await StreamRoom.pick()
    goLiveUi.picked = picked
    goLiveUi.channelId = channelId
  } catch (e) {
    // cancelling the picker is not an error
    if ((e as Error)?.name !== 'NotAllowedError') {
      goLiveUi.error = errorText(e)
      goLiveUi.channelId = channelId
    }
  }
}

export function closeGoLive() {
  goLiveUi.picked = null
  goLiveUi.channelId = null
}
