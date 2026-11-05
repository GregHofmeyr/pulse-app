// Group DM icons (#56): "Change icon" in a group's conversation menu picks a picture, the crop dialog
// (GroupIconCrop, in the shell) frames it, and it goes through the avatar upload; any member may. The server
// tells every member with a ChannelUpdated.
import type { CropRect } from './crop'
import { ui } from './store.svelte'
import { api, errorText } from './tauri'

/** The pictures the avatar pipeline takes (as Settings → Profile). */
export const ICON_ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'

/** A picture picked for a group, waiting for its crop. */
export const groupIcon = $state({ pending: null as { channelId: string; file: File } | null })

/** Pick a picture for group `channelId`: the system file picker, or `file` when it's already chosen. */
export function pickGroupIcon(channelId: string, file?: File): void {
  if (file) {
    groupIcon.pending = { channelId, file }
    return
  }
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = ICON_ACCEPT
  input.addEventListener('change', () => {
    const f = input.files?.[0]
    if (f) groupIcon.pending = { channelId, file: f }
  })
  input.click()
}

/** Upload the pending picture with `crop` and make it the group's icon. A refusal shows as a notice. */
export async function applyGroupIcon(crop: CropRect): Promise<void> {
  const p = groupIcon.pending
  groupIcon.pending = null
  if (!p) return
  try {
    const info = await api.uploadAvatar(new Uint8Array(await p.file.arrayBuffer()), crop)
    await api.setGroupIcon(p.channelId, info.hash)
  } catch (e) {
    ui.notice = errorText(e)
  }
}

export function cancelGroupIcon(): void {
  groupIcon.pending = null
}

/** Back to the members' faces. */
export function removeGroupIcon(channelId: string): Promise<void> {
  return api.setGroupIcon(channelId, null).then(
    () => {},
    (e: unknown) => {
      ui.notice = errorText(e)
    },
  )
}
