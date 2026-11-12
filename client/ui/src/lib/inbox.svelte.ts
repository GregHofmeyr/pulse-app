// The inbox (#306): the unread count on the top bar's button (from Ready, then every inbox event), and the popover's
// lists, one per tab, fetched when first shown and kept live by events. The pure parts are in inbox.ts.
import { appendPage, applyToList, INBOX_PAGE, loadTab, markRead, saveTab, type InboxTab } from './inbox'
import { jumpTo } from './jump'
import type { Event } from './protocol/Event'
import type { InboxItem } from './protocol/InboxItem'
import { app, onAppEvent, onAppReady, ui } from './store.svelte'
import { api, errorText } from './tauri'

export type InboxList = { items: InboxItem[]; end: boolean; loading: boolean; error: string }

export const inbox = $state({
  unread: 0,
  open: false,
  tab: loadTab() as InboxTab,
  /** Each tab's list once fetched (null until then; a Ready forgets them, so a reopened popover fetches again). */
  lists: { all: null, mentions: null } as Record<InboxTab, InboxList | null>,
})

/** Bumped on every Ready: a page fetched before it is dropped. */
let epoch = 0

const TABS: InboxTab[] = ['all', 'mentions']

function setList(tab: InboxTab, list: InboxList | null) {
  inbox.lists = { ...inbox.lists, [tab]: list }
}

/** Fetch the next page of `tab` (the first when it has none yet). One at a time per tab. */
export async function loadMore(tab: InboxTab): Promise<void> {
  const have = inbox.lists[tab]
  if (have?.loading || have?.end) return
  const mine = epoch
  const before = have?.items[have.items.length - 1]?.message.id ?? null
  setList(tab, { items: have?.items ?? [], end: false, loading: true, error: '' })
  try {
    const page = await api.inbox(before, tab === 'mentions')
    if (mine !== epoch) return
    inbox.unread = page.unread
    const now = inbox.lists[tab]
    setList(tab, { items: appendPage(now?.items ?? [], page.items), end: page.items.length < INBOX_PAGE, loading: false, error: '' })
  } catch (e) {
    if (mine !== epoch) return
    setList(tab, { items: inbox.lists[tab]?.items ?? [], end: false, loading: false, error: errorText(e) })
  }
}

/** Ask the server for the count again (something changed it without an inbox event: a chat or server deleted). */
export async function refreshCount(): Promise<void> {
  const mine = epoch
  try {
    const n = await api.inboxUnread()
    if (mine === epoch) inbox.unread = n
  } catch {
    /* the next event or Ready has it */
  }
}

export function openInbox(): void {
  inbox.open = true
  if (!inbox.lists[inbox.tab]) void loadMore(inbox.tab)
  else void refreshCount()
}

export function closeInbox(): void {
  inbox.open = false
}

export function toggleInbox(): void {
  if (inbox.open) closeInbox()
  else openInbox()
}

export function setTab(tab: InboxTab): void {
  inbox.tab = tab
  saveTab(tab)
  if (!inbox.lists[tab]) void loadMore(tab)
}

/** Mark an item read here at once, and on the server (whose event then tells every device the count). */
function readHere(pick: (i: InboxItem) => boolean) {
  for (const tab of TABS) {
    const list = inbox.lists[tab]
    if (!list) continue
    const items = markRead(list.items, pick)
    if (items !== list.items) setList(tab, { ...list, items })
  }
}

/** Open an item: show the message (a reply: its quote's replies panel; a clip's reply: its clips channel) and read
 *  it. The popover closes. */
export function openItem(item: InboxItem): void {
  const m = item.message
  closeInbox()
  if (!item.read) {
    readHere((i) => i.message.id === m.id)
    inbox.unread = Math.max(0, inbox.unread - 1)
    api.inboxRead(m.id).catch(() => void refreshCount())
  }
  if (!m.thread_root_id) return void jumpTo(m.channel_id, m.id)
  const ch = app.state.channels[m.channel_id]
  if (!ch) return void (ui.notice = 'That channel was deleted.')
  ui.goTo = m.channel_id
  if (ch.kind === 'quotes') ui.openReplies = { channelId: m.channel_id, rootId: m.thread_root_id }
}

/** Mark all read: everything up to the newest item shown (or all of it, when none is). */
export async function markAllRead(): Promise<void> {
  const newest = TABS.map((t) => inbox.lists[t]?.items[0]?.message.id ?? '').reduce((a, b) => (b > a ? b : a), '')
  readHere((i) => !newest || i.message.id <= newest)
  inbox.unread = 0
  try {
    await api.inboxReadAll(newest || null)
  } catch {
    void refreshCount()
  }
}

/** A live event: inbox events, and the read points, seen points and deletes that read or remove items. */
export function noteInboxEvent(e: Event): void {
  if (e.t === 'InboxItemAdded' || e.t === 'InboxUpdated') inbox.unread = e.d.unread
  for (const tab of TABS) {
    const list = inbox.lists[tab]
    if (!list) continue
    const items = applyToList(list.items, e, tab)
    if (items !== list.items) setList(tab, { ...list, items })
  }
  // a mute hides (or an unmute brings back) plain replies in that chat: the lists are fetched again
  if (e.t === 'MutesChanged') {
    epoch++
    inbox.lists = { all: null, mentions: null }
    if (inbox.open) void loadMore(inbox.tab)
  }
  // gone or back without an inbox event: the count comes from the server, the lists drop what's no longer here
  if (e.t === 'ChannelDeleted' || e.t === 'ServerDeleted' || e.t === 'ChannelRemoved' || e.t === 'ServerRestored' || e.t === 'ChannelRestored') {
    void refreshCount()
    for (const tab of TABS) {
      const list = inbox.lists[tab]
      if (!list) continue
      const items = list.items.filter((i) => !!app.state.channels[i.message.channel_id])
      if (items.length !== list.items.length) setList(tab, { ...list, items })
    }
  }
}

/** A (re)connected snapshot: its count, and the lists fetched again (the open one now, the other when shown). */
export function noteInboxReady(unread: number | undefined): void {
  epoch++
  inbox.unread = unread ?? 0
  inbox.lists = { all: null, mentions: null }
  if (inbox.open) void loadMore(inbox.tab)
}

onAppReady((r) => noteInboxReady(r.inbox_unread))
onAppEvent(noteInboxEvent)
