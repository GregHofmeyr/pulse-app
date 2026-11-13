// Download a clip (#204), save an image (#206) or a message's file (#57): the menus' Download / Save image and the
// icons and buttons all come here. The Rust core opens the Save dialog and streams the original file to disk,
// reporting progress (#205, `downloads`); a cancelled dialog is a quiet `false`. One per file at a time: a second
// click while its dialog is open does nothing, and while it downloads it cancels it.
import { api } from './tauri'
import { clipKey, downloads, fileKey, imageKey } from './downloads.svelte'
import type { Attachment } from './protocol/Attachment'
import type { LineMedia } from './protocol/LineMedia'
import type { Message } from './protocol/Message'

export function downloadClip(message: Message, onError: (text: string) => void): Promise<void> {
  return downloads.toggle(clipKey(message.id), 'clip', () => api.saveClip(message.id, message.content), onError)
}

/** The image's stored file (an animation's full file, not its still frame), named after its caption. */
export function saveImage(media: LineMedia, caption: string | null, onError: (text: string) => void): Promise<void> {
  return downloads.toggle(imageKey(media.hash), 'image', () => api.saveImage(media.hash, caption ?? ''), onError)
}

/** One of a message's files, under its own name. */
export function saveFile(messageId: string, file: Pick<Attachment, 'id' | 'name'>, onError: (text: string) => void): Promise<void> {
  return downloads.toggle(fileKey(file.id), file.name, () => api.saveAttachment(messageId, file.id, file.name), onError)
}
