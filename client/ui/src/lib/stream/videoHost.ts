// `{@attach}` that shows a watch's store-owned `<video>` in a node (grid tile, full view, mini-preview).
// The element moves between hosts instead of being recreated, so the stream doesn't restart.
import type { Attachment } from 'svelte/attachments'
import { videoOf } from './stream.svelte'

export function hostVideo(channelId: string, streamer: string): Attachment<HTMLElement> {
  return (node) => {
    const el = videoOf(channelId, streamer)
    if (!el) return
    node.append(el)
    // moving a <video> in the DOM pauses it
    if (el.paused) void el.play().catch(() => {})
    return () => {
      // the next host may already have taken it
      if (el.parentNode === node) el.remove()
    }
  }
}
