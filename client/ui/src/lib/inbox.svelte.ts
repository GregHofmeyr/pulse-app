// The inbox (#306): the unread count on the top bar's button (from Ready, then every inbox event), and the popover's
// lists, one per tab, fetched when first shown and kept live by events. The pure parts are in inbox.ts.
import { appendPage, applyToList, arrived, INBOX_PAGE, isItemTab, loadTab, markRead, saveTab, type InboxTab, type InboxView } from './inbox'
import { reminders, seeAllReminders } from './reminders.svelte'
import { seeAllThreads } from './inboxThreads.svelte'
import { openMessage } from './openMessage'
import type { Event } from './protocol/Event'
import type { InboxItem } from './protocol/InboxItem'
import { app, onAppEvent, onAppReady, ui } from './store.svelte'
import { api, errorText } from './tauri'

export type InboxList = { items: InboxItem[]; end: boolean; loading: boolean; error: string }

export const inbox = $state({
  unread: 0,
  open: false,
  tab: loadTab() as InboxView,
  /** Each tab's list once fetched (null until then; a Ready forgets them, so a reopened popover fetches again). */
  lists: { all: null, mentions: null } as Record<InboxTab, InboxList | null>,
})

/** The inbox button's count: the server's, which counts threads with replies you haven't seen too (#326, #419). */
export const inboxBadge = (): number => inbox.unread

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
  if (isItemTab(inbox.tab) && !inbox.lists[inbox.tab]) void loadMore(inbox.tab)
  else void refreshCount()
}

export function closeInbox(): void {
  inbox.open = false
}

export function toggleInbox(): void {
  if (inbox.open) closeInbox()
  else openInbox()
}

export function setTab(tab: InboxView): void {
  inbox.tab = tab
  saveTab(tab)
  if (isItemTab(tab) && !inbox.lists[tab]) void loadMore(tab)
}

/** The open tab's next page, when it lists inbox items. */
function loadOpenTab() {
  if (inbox.open && isItemTab(inbox.tab)) void loadMore(inbox.tab)
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

/** Open an item: show the message where it lives (lib/openMessage.ts: a clip, or a clip's reply, opens that clip's
 *  panel; a quote's reply its replies panel; anything else the message in its chat) and read it. The popover closes. */
export function openItem(item: InboxItem): void {
  const m = item.message
  closeInbox()
  if (!item.read) {
    readHere((i) => i.message.id === m.id)
    inbox.unread = Math.max(0, inbox.unread - 1)
    api.inboxRead(m.id).catch(() => void refreshCount())
  }
  if (!openMessage(m)) ui.notice = 'That channel was deleted.'
}

/** Mark all read: everything up to the newest item shown (or all of it, when none is), and every due reminder. */
export async function markAllRead(): Promise<void> {
  const newest = TABS.map((t) => inbox.lists[t]?.items[0]?.message.id ?? '').reduce((a, b) => (b > a ? b : a), '')
  readHere((i) => !newest || i.message.id <= newest)
  seeAllReminders()
  const threadsSeen = seeAllThreads()
  inbox.unread = 0
  try {
    // the threads' seen points first, so the count read-all answers with already leaves them out
    await threadsSeen
    await api.inboxReadAll(newest || null)
  } catch {
    void refreshCount()
  }
}

/** A live event: inbox events, and the read points, seen points and deletes that read or remove items. */
export function noteInboxEvent(event: Event): void {
  if (event.t === 'InboxItemAdded' || event.t === 'InboxUpdated') inbox.unread = event.d.unread
  let e = event
  if (e.t === 'InboxItemAdded') {
    // as this device stands: its chat gone (dropped) or already read here (#327)
    const item = arrived(app.state, e.d.item)
    if (!item) return
    e = { ...e, d: { ...e.d, item } }
  }
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
    loadOpenTab()
  }
  // a deleted message's reminders go (#300); one that was unread leaves the count, which the server has
  if (e.t === 'MessageDeleted' && reminders.list.some((r) => r.message_id === e.d.message_id)) {
    const unread = reminders.list.some((r) => r.message_id === e.d.message_id && r.due && !r.seen)
    reminders.list = reminders.list.filter((r) => r.message_id !== e.d.message_id)
    if (unread) void refreshCount()
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
  loadOpenTab()
}

onAppReady((r) => noteInboxReady(r.inbox_unread))
onAppEvent(noteInboxEvent)
