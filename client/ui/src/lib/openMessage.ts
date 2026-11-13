// Where a message opens when it's picked from a list of them (a search result, an inbox item; #324). A clips channel has
// no history to jump in: its clip's panel opens instead (a reply's clip, for a reply). A reply in a quote opens that
// quote's replies, as a notification click does. Anything else is jumped to in its channel's history.
import { jumpTo } from './jump'
import type { Message } from './protocol/Message'
import { app, ui } from './store.svelte'
import { api, errorText } from './tauri'

/** Bumped by every open: a clip fetched for an older one is dropped. */
let seq = 0

/** Open `m` where it lives. `root` is its quote or clip when it's a reply and the caller has it (a reply's clip not
 *  given, nor held here, is fetched first). False when its channel isn't known here (nothing opens). */
export function openMessage(m: Message, root: Message | null = null): boolean {
  const ch = app.state.channels[m.channel_id]
  if (!ch) return false
  const mine = ++seq
  if (ch.kind === 'clips') {
    ui.goTo = ch.id
    const rootId = m.thread_root_id
    const clip = !rootId ? m : root?.id === rootId ? root : heldClip(ch.id, rootId)
    if (clip) ui.openClip = { channelId: ch.id, clip }
    else if (rootId) void fetchClip(ch.id, rootId, mine)
    return true
  }
  if (m.thread_root_id) {
    ui.goTo = ch.id
    ui.openReplies = { channelId: ch.id, rootId: m.thread_root_id }
    return true
  }
  void jumpTo(ch.id, m.id)
  return true
}

/** A clip of `channelId` in one of its loaded feeds. */
function heldClip(channelId: string, id: string): Message | null {
  for (const feed of Object.values(app.state.clipFeeds)) {
    if (feed.channelId !== channelId) continue
    const clip = feed.items.find((c) => c.id === id)
    if (clip) return clip
  }
  return null
}

/** Fetch a clip (the window around it in its channel), then open its panel: unless another open took over, or you
 *  went elsewhere meanwhile. */
async function fetchClip(channelId: string, id: string, mine: number): Promise<void> {
  const current = () => mine === seq && (ui.goTo === channelId || ui.openChannelId === channelId)
  let page: Message[] | null
  try {
    page = await api.listMessagesAround(channelId, id)
  } catch (e) {
    if (current()) ui.notice = `Couldn't open that clip: ${errorText(e)}`
    return
  }
  if (!current()) return
  const clip = page?.find((c) => c.id === id)
  if (!clip || clip.deleted) return void (ui.notice = 'That clip was deleted.')
  ui.openClip = { channelId, clip }
}
