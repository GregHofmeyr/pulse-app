// Jump to a message anywhere in history: switch to its channel, load the window around it if it
// isn't loaded, and ask the channel view to scroll to it and flash it (`ui.jump`). Search, mentions
// and pins can call this the same way.
import { loadWindow } from './state'
import { app, ui } from './store.svelte'
import { api } from './tauri'

let seq = 0

export async function jumpTo(channelId: string, messageId: string): Promise<void> {
  if (!app.state.channels[channelId]) return
  const mine = ++seq
  ui.goTo = channelId
  if (!app.state.messages[channelId]?.some((m) => m.id === messageId)) {
    let page
    try {
      page = await api.listMessagesAround(channelId, messageId)
    } catch {
      // Deleted, never in this channel, or a reply (replies live in their thread): say so.
      if (mine === seq) ui.jump = { channelId, messageId, seq: mine, missing: true }
      return
    }
    if (mine !== seq) return // a newer jump took over
    app.state = loadWindow(app.state, channelId, page, messageId)
  }
  ui.jump = { channelId, messageId, seq: mine }
}
