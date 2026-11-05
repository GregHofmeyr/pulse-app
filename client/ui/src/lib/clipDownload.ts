// Download a clip (#204): the clip menu's Download and the icon on the video both come here. The Rust core
// opens the Save dialog and streams the original file to disk; a cancelled dialog is a quiet `false`.
import { api, errorText } from './tauri'
import type { Message } from './protocol/Message'

// One Save dialog per clip at a time: a second click while it is open (or saving) does nothing.
const saving = new Set<string>()

export async function downloadClip(message: Message, onError: (text: string) => void): Promise<void> {
  if (saving.has(message.id)) return
  saving.add(message.id)
  try {
    await api.saveClip(message.id, message.content)
  } catch (e) {
    onError(errorText(e))
  } finally {
    saving.delete(message.id)
  }
}
