// Jump to a message anywhere in history: switch to its channel, load the window around it if it
// isn't loaded, and ask the channel view to scroll to it and flash it (`ui.jump`). Search, mentions
// and pins can call this the same way. "Jump to present" goes back to the latest; the newest of the
// two requests wins.
import { appendNewer, loadLatest, loadWindow } from './state'
import { app, ui } from './store.svelte'
import { api, errorText } from './tauri'

let seq = 0

/** Still the newest request, and `channelId` still the one being shown (or about to be). */
const current = (mine: number, channelId: string) => mine === seq && (ui.goTo === channelId || ui.openChannelId === channelId)

export async function jumpTo(channelId: string, messageId: string): Promise<void> {
  if (!app.state.channels[channelId]) return
  const mine = ++seq
  ui.goTo = channelId
  if (!app.state.messages[channelId]?.some((m) => m.id === messageId)) {
    let page
    try {
      page = await api.listMessagesAround(channelId, messageId)
    } catch (e) {
      if (current(mine, channelId)) ui.jump = { channelId, messageId, seq: mine, error: errorText(e) }
      return
    }
    if (!current(mine, channelId)) return // a newer request took over, or you went elsewhere
    // Deleted, never in this channel, or a reply (replies live in their thread): say so.
    if (!page) return void (ui.jump = { channelId, messageId, seq: mine, missing: true })
    app.state = loadWindow(app.state, channelId, page, messageId)
  }
  ui.jump = { channelId, messageId, seq: mine }
}

/** Most `after` pages fetched to catch up on live messages that arrived while the latest loaded. */
const CATCH_UP_PAGES = 5

/** Reload `channelId`'s latest page, attaching its list again. False when a newer jump took over
 * (nothing to scroll to). Throws if the request fails. */
export async function jumpToPresent(channelId: string): Promise<boolean> {
  const mine = ++seq
  const page = await api.listMessages(channelId)
  if (mine !== seq) return false
  app.state = loadLatest(app.state, channelId, page)
  // A live message newer than the page arrived meanwhile (your own, sent from the window): fetch it.
  for (let i = 0; i < CATCH_UP_PAGES && app.state.history[channelId]?.end === false; i++) {
    const list = app.state.messages[channelId] ?? []
    const newest = list[list.length - 1]?.id
    if (!newest) break
    const more = await api.listMessagesAfter(channelId, newest)
    if (mine !== seq) return false
    app.state = appendNewer(app.state, channelId, newest, more)
  }
  return true
}
