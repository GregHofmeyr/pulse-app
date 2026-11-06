// Download a clip (#204) or save an image (#206): the menus' Download / Save image and the icon on the video or
// picture all come here. The Rust core opens the Save dialog and streams the original file to disk; a cancelled
// dialog is a quiet `false`.
import { api, errorText } from './tauri'
import type { LineMedia } from './protocol/LineMedia'
import type { Message } from './protocol/Message'

// One Save dialog per file at a time: a second click while it is open (or saving) does nothing.
const saving = new Set<string>()

async function once(key: string, save: () => Promise<unknown>, onError: (text: string) => void): Promise<void> {
  if (saving.has(key)) return
  saving.add(key)
  try {
    await save()
  } catch (e) {
    onError(errorText(e))
  } finally {
    saving.delete(key)
  }
}

export function downloadClip(message: Message, onError: (text: string) => void): Promise<void> {
  return once(`clip:${message.id}`, () => api.saveClip(message.id, message.content), onError)
}

/** The image's stored file (an animation's full file, not its still frame), named after its caption. */
export function saveImage(media: LineMedia, caption: string | null, onError: (text: string) => void): Promise<void> {
  return once(`image:${media.hash}`, () => api.saveImage(media.hash, caption ?? ''), onError)
}
