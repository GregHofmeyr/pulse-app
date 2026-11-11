<script lang="ts">
  import { flushSync, onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import MessageItem from './MessageItem.svelte'
  import QuoteComposer from './QuoteComposer.svelte'
  import QuoteItem from './QuoteItem.svelte'
  import QotyCard from './QotyCard.svelte'
  import RepliesPanel from './RepliesPanel.svelte'
  import Leaderboard from './Leaderboard.svelte'
  import PinsPopover from './PinsPopover.svelte'
  import Composer from './Composer.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import ConversationMenu from './ConversationMenu.svelte'
  import GroupIcon from './GroupIcon.svelte'
  import CallBand from './CallBand.svelte'
  import { startCall } from '../lib/calls.svelte'
  import { voice } from '../lib/voice.svelte'
  import SearchBox from './SearchBox.svelte'
  import { narrowerThan } from '../lib/actions'
  import { pinToBottom } from '../lib/pinBottom'
  import { lastInput } from '../lib/inputModality'
  import { app, clock, queueMessage, ui } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import { motion } from '../lib/animate.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { conversationName, firstUnreadIndex, isMuted, reanchor, shouldMarkRead } from '../lib/conversations'
  import { addHistory, addOlder, appendNewer, markHistory, reachLatest, PAGE, pendingMessage, removePending } from '../lib/state'
  import { jumpTo, jumpToPresent } from '../lib/jump'
  import { pinSync, refreshPins } from '../lib/pinSync.svelte'
  import { liveQoty } from '../lib/quotes'
  import { displayName, groupedPending, groupedWith, keepStable, mentionablesFor, nameOfIn, pillNamesFor, nextTypingExpiry, typingNames } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'
  import type { Gif } from '../lib/protocol/Gif'
  import { takeChatFocus, voiceChat } from '../lib/voiceChat.svelte'
  import { COMMANDS, parseCommand, type CommandContext } from '../lib/commands'
  import QuietSpinCard from './QuietSpinCard.svelte'
  import { onOwnSpin, provideChatShown, quietSpins, spinQuick } from '../lib/wheels.svelte'

  let { channelId, serverId, panel = null, onAddPeople = () => {}, onLeft = () => {}, onStartQoty = () => {}, onVoiceSettings = () => {} }: {
    /** A voice channel's chat (#226), as the panel beside its stage: a Chat header with a close button, no search.
     *  `shown`: false while the column is hidden (full view, a narrow window's search), when nothing is marked read. */
    panel?: { onClose: () => void; shown: boolean } | null
    /** DMs/groups: the call band's person menu has Voice settings (#50). */
    onVoiceSettings?: () => void
    channelId: string
    serverId: string | null
    /** DMs/groups: open the people picker. */
    onAddPeople?: () => void
    /** After leaving/closing this conversation. */
    onLeft?: () => void
    /** Quotes channels: start Quote of the Year (the button hides while a vote is live). */
    onStartQoty?: () => void
  } = $props()

  const channel = $derived(app.state.channels[channelId])
  const messages = $derived(app.state.messages[channelId] ?? [])
  const pending = $derived((app.state.pending[channelId] ?? []).filter((p) => !p.thread_root_id)) // a reply's pending shows in its thread
  const pendingMessages = $derived(pending.map((p) => pendingMessage(p, channelId, app.state.me?.id ?? '')))
  const byId = $derived(new Map(messages.map((m) => [m.id, m])))
  let now = $state(Date.now())
  const typers = $derived(typingNames(app.state, serverId, channelId, now))
  let replyTo = $state<Message | null>(null)
  let error = $state('')
  let list: HTMLDivElement
  let composer: Composer | undefined = $state()
  let loadingOlder = false
  let fetching: string | null = null
  const history = $derived(app.state.history[channelId])
  const reachedStart = $derived(!!history?.start)
  /** The list is a window into the past (a jump), not the latest. */
  const detached = $derived(history?.end === false)
  /** Quiet spins on show here (#270): cards at the bottom, while the list is the latest (not a jump's window). */
  const quiet = $derived(quietSpins[channelId] ?? [])

  // Fade typing indicators on time, not just on events: one timer for the next expiry, and only
  // while someone is typing (an always-on 1 s interval woke the app all day).
  $effect(() => {
    const next = nextTypingExpiry(app.state, channelId, now)
    if (next === null) return
    const t = setTimeout(() => (now = Date.now()), Math.max(0, next - Date.now()) + 50)
    return () => clearTimeout(t)
  })

  $effect(() => {
    void channelId
    replyTo = null
  })

  // Fetch the latest page whenever this channel isn't marked loaded (first view, or after a
  // reconnect cleared the markers). Marked only on success, so an offline failure retries later.
  $effect(() => {
    const c = channelId
    if (history?.loaded || fetching === c) return
    fetching = c
    api.listMessages(c).then(
      async (page) => {
        fetching = null
        // A jump's window got there first: merging the latest page into it would leave a gap.
        if (app.state.history[c]?.loaded) return
        app.state = markHistory(addHistory(app.state, c, page), c, page.length < PAGE)
        await tick()
        list?.scrollTo({ top: list.scrollHeight })
      },
      (e) => {
        fetching = null
        error = errorText(e)
      },
    )
  })

  // A reconnect fetches only the latest page again: the older pages (or a jump's window) still loaded keep the pins
  // they had, so the chat's pins are fetched to set them right (#258).
  $effect(() => {
    if (pinSync.stale[channelId]) void refreshPins(channelId)
  })

  // Coming back to a channel whose history is still loaded: nothing is fetched, so nothing above
  // scrolls, and a fresh list opens at the top. Open at the newest, like a first visit (a jump that
  // brought you here scrolls to its message itself).
  onMount(() => {
    if (!untrack(() => history?.loaded) || ui.jump?.channelId === channelId) return
    void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  })

  // Opened with the pointer (a click on the channel or conversation): you're here to write, so the
  // message box takes the focus, the caret after any draft. Not from the keyboard (arrows or Enter in
  // a list keep you in the list), and not under an open dialog or menu.
  // The voice chat panel doesn't: showing the stage brings it back open, and that's no click on the chat. Its own
  // openers (the row's chat icon, the stage's toggle) ask for the focus instead, below.
  function focusOnClick() {
    if (lastInput() !== 'pointer' || document.querySelector('[aria-modal="true"], [role="dialog"], [role="menu"]')) return
    void tick().then(() => {
      if (!chan?.isConnected) return
      if (composer) return composer.focus(true)
      const q = chan.querySelector<HTMLTextAreaElement>('.new-quote:not([hidden]) textarea.qtext')
      q?.focus()
      q?.setSelectionRange(q.value.length, q.value.length)
    })
  }
  onMount(() => {
    if (!panel) focusOnClick()
  })
  $effect(() => {
    void voiceChat.focusWanted
    if (panel && untrack(takeChatFocus)) focusOnClick()
  })

  // stick to the bottom when new messages arrive and we were already near it
  $effect(() => {
    void messages.length
    void pending.length
    void quiet.length
    // A detached window's bottom isn't the latest, and a page added below one stays below the view.
    if (!list || loadingNewer || untrack(() => detached)) return
    const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 120
    if (nearBottom) tick().then(() => list?.scrollTo({ top: list.scrollHeight })) // gone if we unmounted meanwhile
  })

  // ...and stay there as messages grow after they're drawn (#292): a link's card unfurling later, images loading,
  // reactions, a poll or spin card changing. Not in a detached window (its bottom isn't the latest) or while a newer
  // page goes in below the view.
  let pin: ReturnType<typeof pinToBottom> | undefined
  $effect(() => {
    if (!list) return
    const p = (pin = pinToBottom(list, { on: () => !detached && !loadingNewer }))
    return () => p.destroy()
  })

  // Opening cheaply (#238): a channel opens with only its newest messages, enough to fill the list, and draws the rest
  // of its loaded history after that first frame, a frame's worth at a time, older ones going in above the view while
  // it stays still. So a switch shows the chat without first mounting every message in it, and no one task mounts
  // them all later either. The first few are measured: too few to fill the list, as many more as should (once), else
  // all of them at once, so the view starts at the bottom exactly as it ends up. A jump (or a notification's replies)
  // draws everything first, to find its message.
  const FIRST = 24
  let firstCount = $state(FIRST)
  let drawnAll = $state(untrack(() => ui.jump?.channelId === channelId || ui.openReplies?.channelId === channelId))
  const from = $derived(drawnAll ? 0 : Math.max(0, messages.length - firstCount))
  const shown = $derived(from ? messages.slice(from) : messages)
  let drawTimer: ReturnType<typeof setTimeout> | undefined
  let drawFrame = 0
  const nextFrame = () => (drawFrame = requestAnimationFrame(() => (drawTimer = setTimeout(drawMore))))
  /** Draws the next older ones, the view kept still (WebKitGTK doesn't anchor scrolling; #173): at the bottom the
   *  newest stays where it is on screen, else the top one in view. Then the next after the coming frame, until all
   *  are in. */
  function drawMore() {
    if (drawnAll) return
    const drawnNow = list ? [...list.querySelectorAll<HTMLElement>('[data-message-id]')] : []
    const top = list?.getBoundingClientRect().top ?? 0
    const anchor = atBottom ? drawnNow.at(-1) : drawnNow.find((el) => el.getBoundingClientRect().bottom > top)
    const y0 = anchor?.getBoundingClientRect().top ?? 0
    const fromBottom = list ? list.scrollHeight - list.scrollTop : 0
    firstCount += FIRST
    if (firstCount >= messages.length) drawnAll = true
    flushSync()
    if (list && anchor?.isConnected) list.scrollTop += anchor.getBoundingClientRect().top - y0
    else if (list) list.scrollTop = list.scrollHeight - fromBottom
    if (!drawnAll) nextFrame()
  }
  $effect(() => {
    if (drawnAll || !messages.length) return
    const partial = from > 0
    untrack(() => {
      if (!partial || !list) return void (drawnAll = true) // all there is
      if (drawFrame) return // drawing the rest
      // measured with only the newest in the list (the layout this frame needs anyway)
      const short = list.scrollHeight <= list.clientHeight
      if (short && firstCount === FIRST) return void (firstCount = Math.ceil((firstCount * (list.clientHeight + 200)) / Math.max(1, list.scrollHeight)))
      if (short) return void (drawnAll = true)
      nextFrame()
    })
  })
  $effect(() => () => {
    cancelAnimationFrame(drawFrame)
    clearTimeout(drawTimer)
  })

  async function maybeLoadOlder() {
    if (loadingOlder || reachedStart || !drawnAll || list.scrollTop > 60 || !messages.length) return
    loadingOlder = true
    try {
      const oldest = messages[0].id
      const page = await api.listMessages(channelId, oldest)
      // Keep the view still (unless we left): the oldest message stays where it is on screen once the page is drawn
      // above it, measured after the wait so a scroll meanwhile counts, and whether or not the engine anchors
      // scrolling itself (WebKitGTK doesn't; #173).
      const anchor = list?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(oldest)}"]`)
      const y0 = anchor?.getBoundingClientRect().top ?? 0
      const before = list?.scrollHeight ?? 0
      app.state = addOlder(app.state, channelId, oldest, page)
      await tick()
      if (list && anchor?.isConnected) list.scrollTop += anchor.getBoundingClientRect().top - y0
      else if (list) list.scrollTop += list.scrollHeight - before // no element to hold on to: by the height added
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingOlder = false
    }
  }

  // Who can be @mentioned here: a DM/group's members, or everyone for server channels.
  const mentionables = $derived(mentionablesFor(app.state, channelId))
  // Same names → same array, so an unrelated event (typing, presence) doesn't re-render every message.
  const sameNames = keepStable<string>()
  // mentions are written as usernames; pills then show the name that applies (MessageItem)
  const mentionNames = $derived(sameNames(pillNamesFor(app.state, channelId)))

  // --- DM/group header ---
  const isPrivate = $derived(channel?.server_id === null)
  const isVoice = $derived(channel?.kind === 'voice')
  const title = $derived(isPrivate ? conversationName(app.state, channelId) : (channel?.name ?? ''))
  const others = $derived((app.state.dmMembers[channelId] ?? []).filter((u) => u !== app.state.me?.id))
  const muted = $derived(channel ? isMuted(app.state, channel, clock.nowIso) : false)
  // the phone button (#50): start this conversation's call (ringing the others), or join the one going on
  const callGoing = $derived(!!app.state.calls[channelId])
  const inThisCall = $derived(voice.channelId === channelId || voice.joining === channelId)
  // "Share to the stage?" on a Spotify link in the box (#288): a server voice channel's chat while you're in its call,
  // as a Spotify card's Share to stage (LinkEmbed).
  const stageNudge = $derived(
    isVoice && !!channel?.server_id && voice.channelId === channelId && voice.connection === 'connected'
      ? { share: (url: string) => api.shareToStage(channelId, url).then(() => {}), up: (app.state.stageShares[channelId] ?? []).map((x) => x.embed.url) }
      : null,
  )
  async function call() {
    error = ''
    try {
      await startCall(channelId)
    } catch (e) {
      error = errorText(e)
    }
  }
  let menu = $state<null | 'mute' | 'manage'>(null)
  // the buttons the menu opens by (#162)
  let muteBtn = $state<HTMLButtonElement>()
  let moreBtn = $state<HTMLButtonElement>()
  let renaming = $state(false)
  let renameValue = $state('')
  function startRename() {
    renameValue = channel?.name ?? ''
    renaming = true
  }
  async function saveRename() {
    if (!renaming) return
    renaming = false
    const v = renameValue.trim()
    if (v === (channel?.name ?? '')) return
    try {
      await api.renameChannel(channelId, v || null)
    } catch (e) {
      error = errorText(e)
    }
  }

  // --- unread: the NEW line starts at the read point you opened with, then moves to the newest
  // message each time you stop watching (unfocus / scroll up), so every away stretch gets one ---
  let anchor = $state(untrack(() => app.state.reads[channelId]?.last_read_message_id ?? null))
  let anchored = $state(untrack(() => channelId in app.state.reads)) // brand-new chats start without a line
  const newIndex = $derived(anchored ? firstUnreadIndex(messages, anchor, app.state.me?.id ?? '') : -1)
  let atBottom = $state(true)
  /** More than a screen above the newest: "Jump to present" shows even with nothing new below. */
  let farUp = $state(false)
  let wasWatching = false
  $effect(() => {
    const now = windowFocus.focused && atBottom
    // A detached window's newest isn't the latest: moving the line there would mark read messages new.
    const newest = untrack(() => (detached ? null : (messages.at(-1)?.id ?? null)))
    const next = reanchor(untrack(() => anchor), { was: wasWatching, now, newest })
    if (next !== untrack(() => anchor)) {
      anchor = next
      anchored = true
    }
    wasWatching = now
  })
  const unreadBelow = $derived(atBottom ? 0 : (app.state.reads[channelId]?.unread ?? 0))
  let markTimer: ReturnType<typeof setTimeout> | null = null
  $effect(() => {
    const newest = messages.at(-1)?.id
    const read = app.state.reads[channelId]?.last_read_message_id ?? null
    const want = shouldMarkRead({ open: panel?.shown ?? true, focused: windowFocus.focused, atBottom })
    // The bottom of a detached window isn't the latest: only reaching the latest marks it read.
    if (!newest || !want || detached || (read !== null && newest <= read)) return
    if (markTimer) clearTimeout(markTimer)
    markTimer = setTimeout(() => void api.markRead(channelId, newest).catch(() => {}), 300)
    return () => {
      if (markTimer) clearTimeout(markTimer)
    }
  })
  function onScroll() {
    const below = list.scrollHeight - list.scrollTop - list.clientHeight
    atBottom = below < 40
    farUp = below > list.clientHeight
    void maybeLoadOlder()
    if (atBottom) void maybeLoadNewer()
  }
  function jump() {
    list.scrollTo({ top: list.scrollHeight, behavior: motion.reducedMotion ? 'auto' : 'smooth' })
  }

  // --- jump to a message (lib/jump.ts sets `ui.jump`): scroll it to the middle and flash it. A
  // detached window pages forward from its bottom, or "Jump to present" reloads the latest. ---
  let loadingNewer = false
  async function maybeLoadNewer(isRetry = false) {
    if (loadingNewer || !detached || !messages.length) return
    loadingNewer = true
    let more = false
    let retry = false
    let missedNow: string[] = []
    try {
      const newest = messages[messages.length - 1].id
      missedNow = [...(history?.missed ?? [])]
      const page = await api.listMessagesAfter(channelId, newest)
      app.state = appendNewer(app.state, channelId, newest, page)
      // An empty page while the state still waits for a missed message (or a dropped older one): ask
      // once more; empty again, nothing is coming, so it is the latest (never a request loop). Only
      // what was already missed when the retry was sent is given up; a message that arrived during
      // it is paged for next (new progress, so the same one-retry rule starts over for it).
      if (!page.length && detached) {
        if (isRetry) {
          app.state = reachLatest(app.state, channelId, newest, missedNow)
          more = detached
        } else retry = true
      }
      await tick() // with loadingNewer still set, the view stays where it was
      pin?.mark() // and stays there once it's done: the page went in below the view, not under it
      // Still at the bottom (measured now: atBottom is from the last scroll), e.g. a short page that
      // stopped before a missed live message: fetch on, but only while pages bring something.
      more = more || (page.length > 0 && detached && !!list && list.scrollHeight - list.scrollTop - list.clientHeight < 40)
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingNewer = false
    }
    if (more) void maybeLoadNewer()
    else if (retry) void maybeLoadNewer(true)
  }
  let loadingPresent = false
  async function toPresent() {
    if (loadingPresent) return
    loadingPresent = true
    try {
      if (!(await jumpToPresent(channelId))) return // a jump took over
      await tick()
      list?.scrollTo({ top: list.scrollHeight })
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingPresent = false
    }
  }
  const FLASH_MS = 2000
  let flashEl: Element | null = null
  let flashTimer: ReturnType<typeof setTimeout> | undefined
  let noticeTimer: ReturnType<typeof setTimeout> | undefined
  let jumpNotice = $state('')
  function clearFlash() {
    clearTimeout(flashTimer)
    flashEl?.classList.remove('jump-flash', 'still')
    flashEl = null
  }
  function flash(id: string) {
    const el = list?.querySelector(`[data-message-id="${id}"]`)
    if (!el) return
    clearFlash()
    void (el as HTMLElement).offsetWidth // a repeat restarts the animation
    el.classList.add('jump-flash')
    if (motion.reducedMotion) el.classList.add('still')
    el.scrollIntoView({ block: 'center' })
    flashEl = el
    flashTimer = setTimeout(clearFlash, FLASH_MS)
  }
  $effect(() => {
    const j = ui.jump
    if (!j || j.channelId !== channelId) return
    if (j.missing || j.error) {
      ui.jump = null
      jumpNotice = j.error ?? "Couldn't find that message"
      clearTimeout(noticeTimer)
      noticeTimer = setTimeout(() => (jumpNotice = ''), 4000)
      return
    }
    if (!messages.some((m) => m.id === j.messageId)) return // its window is still loading
    ui.jump = null
    jumpNotice = ''
    drawnAll = true // its message may be among the rest; the flash scrolls to it
    void tick().then(() => flash(j.messageId))
  })
  $effect(() => () => {
    clearFlash()
    clearTimeout(noticeTimer)
    // a request for this channel that never got handled must not fire when it's next opened
    untrack(() => {
      if (ui.jump?.channelId === channelId) ui.jump = null
    })
  })

  // --- quotes channels ---
  const isQuotes = $derived(channel?.kind === 'quotes')
  // The quote whose Edit was pressed; the quote composer opens on it.
  let editingQuote = $state<Message | null>(null)
  $effect(() => {
    void channelId
    editingQuote = null
  })

  // The replies panel (one at a time: opening another replaces it) and the leaderboard popover.
  let repliesOpen = $state<Message | null>(null)
  // Kept current: the loaded copy, else the store's copy of the open quote (it may leave the list).
  const repliesRoot = $derived(
    repliesOpen ? (byId.get(repliesOpen.id) ?? (app.state.openRoot?.id === repliesOpen.id ? app.state.openRoot : repliesOpen)) : null,
  )
  // What opened the panel gets the focus back when you close it (Escape or ✕).
  let repliesOpener: HTMLElement | null = null
  function openReplies(m: Message) {
    const at = document.activeElement
    repliesOpener = at instanceof HTMLElement && list?.contains(at) ? at : null
    repliesOpen = m
  }
  // A clicked notification for a reply asked for its quote's panel: open it once the quote is in
  // the list, fetching the history around it first if it isn't (once per request).
  let fetchedFor: string | null = null
  $effect(() => {
    const r = ui.openReplies
    if (!r || r.channelId !== channelId) return
    const root = byId.get(r.rootId)
    untrack(() => {
      if (root) {
        ui.openReplies = null
        fetchedFor = null
        openReplies(root)
      } else if (fetchedFor !== r.rootId) {
        fetchedFor = r.rootId
        void jumpTo(channelId, r.rootId)
      }
    })
  })
  $effect(() => () => {
    untrack(() => {
      if (ui.openReplies?.channelId === channelId) ui.openReplies = null
    })
  })
  let chan: HTMLDivElement
  async function closeReplies() {
    const id = repliesOpen?.id
    const opener = repliesOpener
    // Asked before the panel goes: closing it lets go of the quote it held (see editedAlive).
    const editAlive = editedAlive()
    repliesOpen = null
    repliesOpener = null
    await tick()
    // The opener; else (a click that didn't focus it, or it's gone since, like the unsent-reply marker
    // while its panel was open) the quote's marker or first line; else (the quote is gone) the new-quote
    // box; else (an edit hides it) that edit box if its quote is still there; else the message list.
    const quote = `[data-message-id="${id}"]`
    const back = (opener?.isConnected ? opener : null)
      ?? list?.querySelector<HTMLElement>(`${quote} .unsent-reply`)
      ?? list?.querySelector<HTMLElement>(`${quote} .line`)
      ?? chan?.querySelector<HTMLElement>('.new-quote:not([hidden]) textarea.qtext')
      ?? (editAlive ? chan?.querySelector<HTMLElement>('.edit-quote textarea.qtext') : null)
      ?? list
    back?.focus()
  }
  function editedAlive() {
    // Like the composer's live copy: the list's, else the panel's held one (it may have left the list).
    const q = editingQuote && (byId.get(editingQuote.id) ?? (app.state.openRoot?.id === editingQuote.id ? app.state.openRoot : editingQuote))
    return !!q && !q.deleted
  }
  // An edit needs a live copy of its quote (award counts, M19): the list's, or the open panel's held one.
  // Once neither holds it (its panel closed, another panel replaced it, or a jump took it out of the
  // list), the edit ends rather than run on a frozen copy, as leaving the channel ends it. The focus,
  // if it was in that edit, goes to the new-quote box.
  $effect(() => {
    const e = editingQuote
    if (!e || byId.has(e.id) || app.state.openRoot?.id === e.id) return
    untrack(() => {
      const inEdit = !!document.activeElement?.closest('.edit-quote') || document.activeElement === document.body
      editingQuote = null
      if (inEdit) void tick().then(() => chan?.querySelector<HTMLElement>('.new-quote:not([hidden]) textarea.qtext')?.focus())
    })
  })
  // A deleted quote's thread is gone: its panel closes (unsent replies to it were dropped with it). The
  // focus moves on only if it was in the panel; elsewhere it is left alone.
  $effect(() => {
    if (repliesRoot?.deleted) untrack(() => {
      const inPanel = !!document.activeElement?.closest('.replies-panel')
      if (inPanel) void closeReplies()
      else {
        repliesOpen = null
        repliesOpener = null
      }
    })
  })
  // Unsent replies wait in their quote's panel; while it's closed, the quote says it has some. One
  // still sending isn't unsent: replies are only sent from the open panel, and one that can't get
  // through turns failed (after the outbox's tries), so a panel closed mid-send doesn't flash it.
  const unsentReplies = $derived(
    (app.state.pending[channelId] ?? []).reduce<Record<string, number>>((acc, p) => {
      if (p.thread_root_id && p.status === 'failed' && p.thread_root_id !== repliesOpen?.id) acc[p.thread_root_id] = (acc[p.thread_root_id] ?? 0) + 1
      return acc
    }, {}),
  )
  let leaderboardOpen = $state(false)
  const qotyLive = $derived(!!serverId && !!liveQoty(app.state, serverId, Date.parse(clock.nowIso)))
  function openLeaderboardEntry(m: Message) {
    leaderboardOpen = false
    void jumpTo(m.channel_id, m.id)
  }

  // --- pins (#178): the header's pin button, or a pin line's See pins, opens the chat's pins ---
  const pinsOn = $derived(channel?.kind === 'text' || channel?.kind === 'voice' || isPrivate)
  let pinsOpen = $state(false)
  $effect(() => {
    if (ui.openPins !== channelId) return
    untrack(() => {
      ui.openPins = null
      pinsOpen = true
    })
  })
  function openPinned(messageId: string) {
    pinsOpen = false
    void jumpTo(channelId, messageId)
  }

  const nameOf = $derived(nameOfIn(() => app.state, serverId))

  /** To the latest: a page from higher up (a jump's window) goes back to the present, else to the bottom. */
  function toLatest() {
    if (detached) void toPresent()
    else void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  }
  function post(text: string, replyToId: string | null, gif: Gif | null = null) {
    queueMessage(channelId, text, replyToId, gif)
    // Sending from higher up takes you to the latest (your message lands there).
    toLatest()
  }
  // A spin you start (the popover, /wheel, Spin again; public or quiet) lands at the latest too, so you see it
  // spin (#270). Someone else's only follows if you're at the bottom already (the stick-to-bottom effect above).
  onMount(() => onOwnSpin((c) => c === channelId && toLatest()))
  // a spin's card is only seen, and only chimes, while this chat is on screen (#271; a voice chat panel can hide)
  provideChatShown(() => panel?.shown ?? true)
  // what slash commands can do in this chat (lib/commands.ts CommandContext)
  const commandContext: CommandContext = {
    spinWheel: (options) => {
      error = ''
      spinQuick(channelId, options).catch((e) => (error = errorText(e)))
    },
    openWheel: (prefill) => composer?.openWheel(prefill),
    openPoll: (prefill) => composer?.openPoll(prefill),
  }
  function send(text: string) {
    // a slash command (`/wheel a, b, c`, lib/commands.ts) runs instead of being sent (#158, #262)
    const cmd = parseCommand(text)
    if (cmd) {
      cmd.command.run(cmd.args, commandContext)
      return
    }
    const r = replyTo?.id ?? null
    replyTo = null
    post(text, r)
  }
  // a GIF from the picker (#58) goes at once, as a reply to whatever is being replied to
  function sendGif(gif: Gif, link: string) {
    const r = replyTo?.id ?? null
    replyTo = null
    post(link, r, gif)
  }

  // A retry keeps its own target and leaves the one being picked for the next message alone.
  function retry(nonce: string) {
    const p = pending.find((x) => x.nonce === nonce)
    if (!p) return
    app.state = removePending(app.state, channelId, nonce)
    post(p.content, p.reply_to_id, p.gif ?? null)
  }

  // A quote posted from higher up lands at the latest too.
  function quotePosted() {
    if (detached) void toPresent()
  }
  // A narrow header (small window, side panels open): the search box folds to a magnifier (#97).
  const NARROW_HEAD = 640
  let narrowHead = $state(false)
</script>

<div class="chan" class:panel={!!panel} class:with-replies={!!repliesRoot} bind:this={chan}>
  <div class="head" use:narrowerThan={{ px: NARROW_HEAD, set: (v) => (narrowHead = v) }}>
    {#if panel}
      <Icon name="message" size={16} /><span class="title">Chat</span>
    {:else if isPrivate && channel?.kind === 'dm' && others[0]}
      <Avatar id={others[0]} name={title} size={26} presence={presenceOf(app.state, others[0], clock.nowIso)} interactive serverId={null} />
      <span class="title"><Name userId={others[0]} serverId={null} text={title} /></span>
    {:else if isPrivate}
      {#if channel?.icon_hash}
        <GroupIcon hash={channel.icon_hash} still={channel.icon_still} size={26} />
      {:else}
        <span class="pile">
          {#each others.slice(0, 3) as u (u)}<Avatar id={u} name={app.state.people[u] ? displayName(app.state, null, u) : '?'} size={22} />{/each}
        </span>
      {/if}
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input input-sm rename" maxlength="64" placeholder={conversationName({ ...app.state, channels: { ...app.state.channels, [channelId]: { ...channel!, name: null } } }, channelId)}
          bind:value={renameValue} autofocus onblur={saveRename}
          onkeydown={(e) => { if (e.key === 'Enter') void saveRename(); if (e.key === 'Escape') renaming = false }} />
      {:else}
        <button class="title editable" data-tip="Rename group" onclick={startRename}>{title}</button>
      {/if}
    {:else}
      <Icon name={isQuotes ? 'quote' : 'hash'} /> <span class="title">{title}</span>
    {/if}
    <span class="grow"></span>
    {#if isQuotes}
      <button class="icon-btn hbtn" class:on={leaderboardOpen} aria-label="Leaderboard" data-tip="Leaderboard" data-menu-toggle aria-haspopup="dialog" aria-expanded={leaderboardOpen}
        onclick={() => (leaderboardOpen = !leaderboardOpen)}><Icon name="trophy" size={17} /></button>
      {#if !qotyLive}<button class="qoty-btn qoty" onclick={onStartQoty}>Start Quote of the Year</button>{/if}
    {/if}
    {#if isPrivate}
      {#if !inThisCall}
        <button class="icon-btn" aria-label={callGoing ? 'Join call' : 'Start a call'} data-tip={callGoing ? 'Join call' : 'Start a call'} onclick={call}><Icon name="phone" size={17} /></button>
      {/if}
      <button class="icon-btn" aria-label="Add people" data-tip="Add people" onclick={onAddPeople}><Icon name="userPlus" size={17} /></button>
    {/if}
    {#if pinsOn}
      <button class="icon-btn hbtn" class:on={pinsOpen} aria-label="Pinned messages" data-tip="Pinned messages" data-menu-toggle aria-haspopup="dialog" aria-expanded={pinsOpen}
        onclick={() => (pinsOpen = !pinsOpen)}><Icon name="pin" size={17} /></button>
    {/if}
    <button bind:this={muteBtn} class="icon-btn hbtn" class:on={muted} aria-label={muted ? 'Muted' : 'Mute'} data-tip={muted ? 'Muted' : 'Mute'} data-menu-toggle
      aria-haspopup="menu" aria-expanded={menu === 'mute'}
      onclick={() => (menu = menu === 'mute' ? null : 'mute')}><Icon name={muted ? 'bellOff' : 'bell'} size={17} /></button>
    {#if isPrivate}
      <button bind:this={moreBtn} class="icon-btn" aria-label="More" aria-haspopup="menu" aria-expanded={menu === 'manage'} data-tip="More" data-menu-toggle
        onclick={() => (menu = menu === 'manage' ? null : 'manage')}><Icon name="dots" size={17} /></button>
    {/if}
    {#if panel}
      <button class="icon-btn" aria-label="Close chat" data-tip="Close" onclick={panel.onClose}><Icon name="x" size={16} /></button>
    {:else}
      <SearchBox {serverId} {channelId} compact={narrowHead} />
    {/if}
    {#if leaderboardOpen && serverId}
      <Leaderboard {serverId} onOpen={openLeaderboardEntry} onClose={() => (leaderboardOpen = false)} />
    {/if}
    {#if pinsOpen && pinsOn}
      <PinsPopover {channelId} {serverId} onJump={openPinned} onClose={() => (pinsOpen = false)} />
    {/if}
    {#if menu && channel}
      <ConversationMenu {channel} {muted} show={menu} anchor={menu === 'manage' ? moreBtn : muteBtn} onClose={() => (menu = null)} onRename={startRename} {onLeft} />
    {/if}
  </div>
  <!-- not in the chat strip beside a DM or group stream in full view (#51): the call is all around it there -->
  {#if isPrivate && !panel}<CallBand {channelId} {onVoiceSettings} />{/if}
  <!-- tabindex -1: a place to put the focus back when nothing better is left (not a tab stop) -->
  <div class="list" tabindex="-1" bind:this={list} onscroll={onScroll}>
    {#if reachedStart && !from}<div class="start">
      {#if channel?.kind === 'dm'}This is the start of your conversation with {title}.
      {:else if channel?.kind === 'group'}This is the start of {title}.
      {:else if isVoice}This is the start of {title}'s chat.
      {:else}This is the start of #{title}.{/if}
    </div>{/if}
    {#each shown as m, j (m.id)}
      {@const i = from + j}
      {#if i === newIndex}<div class="newline" role="separator" aria-label="New messages">NEW</div>{/if}
      {#if m.qoty_card && serverId}
        <QotyCard message={m} {serverId} quote={m.qoty_card.message_id ? (byId.get(m.qoty_card.message_id) ?? null) : null} />
      {:else if isQuotes && m.quote}
        <QuoteItem message={m} serverId={serverId ?? ''} meId={app.state.me?.id ?? ''} {nameOf} unsentReplies={unsentReplies[m.id] ?? 0}
          onReply={() => openReplies(m)} onOpenReplies={() => openReplies(m)}
          onEdit={() => (editingQuote = m)} onDelete={() => api.deleteMessage(m.id)} />
      {:else}
      <MessageItem message={m} {serverId} name={nameOf(m.author_id)} {nameOf} grouped={groupedWith(messages, i)}
        replyTo={m.reply_to_id ? (byId.get(m.reply_to_id) ?? null) : null}
        mine={m.author_id === app.state.me?.id}
        {mentionNames} meId={app.state.me?.id ?? ''}
        onReply={() => (replyTo = m)}
        onEdit={async (c) => { await api.editMessage(m.id, c) }}
        onDelete={() => api.deleteMessage(m.id)} onEditClosed={() => composer?.focus()} />
      {/if}
    {/each}
    {#each pendingMessages as pm, i (pm.id)}
      {@const p = pending[i]}
      <MessageItem message={pm} {serverId} name={nameOf(app.state.me?.id ?? null)} {nameOf} grouped={groupedPending(messages, pendingMessages, i)}
        replyTo={p.reply_to_id ? (byId.get(p.reply_to_id) ?? null) : null} mine
        {mentionNames} meId={app.state.me?.id ?? ''} pending={p} onReply={() => {}} onEdit={async () => {}} onDelete={async () => {}}
        onRetry={() => retry(p.nonce)} onDiscard={() => (app.state = removePending(app.state, channelId, p.nonce))} />
    {/each}
    {#if !detached}
      {#each quiet as q (q.spin.id)}<QuietSpinCard card={q} {serverId} />{/each}
    {/if}
  </div>
  {#if detached}<button class="jump" onclick={toPresent}>Jump to present</button>
  {:else if unreadBelow > 0}<button class="jump" onclick={jump}>{unreadBelow} new · Jump</button>
  {:else if farUp}<button class="jump" onclick={jump}>Jump to present</button>{/if}
  {#if jumpNotice}<p class="jump-notice" role="status">{jumpNotice}</p>{/if}
  <div class="typing" aria-live="polite">
    {#if typers.length}<strong class="name-clip">{typers.join(', ')}</strong> {typers.length > 1 ? 'are' : 'is'} typing…{/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if isQuotes}
    <!-- the new-quote composer stays mounted (hidden) while one is edited, so its draft survives -->
    {#key channelId}
      <div class="new-quote" hidden={!!editingQuote}><QuoteComposer {channelId} serverId={serverId ?? ''} onDone={quotePosted} /></div>
    {/key}
    {#if editingQuote}
      {#key editingQuote.id}
        <div class="edit-quote"><QuoteComposer {channelId} serverId={serverId ?? ''} editing={editingQuote} onDone={() => (editingQuote = null)} /></div>
      {/key}
    {/if}
  {:else}
  <Composer bind:this={composer} draftKey={channelId} placeholder={isPrivate || isVoice ? `Message ${title}` : `Message #${title}`} replyingTo={replyTo ? nameOf(replyTo.author_id) : null}
    onCancelReply={() => (replyTo = null)} onSend={send} onTyping={() => void api.sendTyping(channelId)} {mentionables} {serverId} wheelChannel={channelId} pollChannel={channelId} commands={COMMANDS} onSendGif={sendGif} {stageNudge} />
  {/if}
  {#if repliesRoot}
    {#key repliesRoot.id}
      <RepliesPanel root={repliesRoot} serverId={serverId ?? ''} onClose={closeReplies} onEditQuote={() => (editingQuote = repliesRoot)} />
    {/key}
  {/if}
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  /* the replies panel sits on the right (RepliesPanel, absolutely placed); the channel keeps the rest */
  .chan { position: relative; --replies-w: min(380px, 45%); }
  .chan.with-replies { padding-right: var(--replies-w); }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .head { position: relative; }
  /* as the voice chat panel (#226): the replies panel's header, a close button at its end */
  .chan.panel .head { padding: 0 12px 0 18px; gap: 8px; }
  .title { color: var(--text); font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .editable { border: 0; background: none; padding: 2px 4px; margin-left: -4px; border-radius: var(--radius-xs); font: inherit; font-size: 15px; font-weight: 600; color: var(--text); }
  .editable:hover { background: var(--bg-3); }
  .rename { min-width: 220px; font-weight: 600; }
  .pile { display: flex; }
  .pile :global(.av) { box-shadow: 0 0 0 2px var(--ring); }
  .pile :global(.av + .av) { margin-left: -7px; }
  .grow { flex: 1; }
  .hbtn.on { background: var(--bg-3); color: var(--text); }
  /* the shared .qoty-btn, at this header's 30px */
  .qoty { height: 30px; padding: 0 12px; }
  .newline { display: flex; align-items: center; gap: 8px; margin: 6px 18px; color: var(--danger); font-size: 11px; font-weight: 700; letter-spacing: .04em; }
  .newline::before, .newline::after { content: ''; flex: 1; height: 1px; background: var(--danger); opacity: .6; }
  .jump { align-self: center; margin-top: -40px; position: relative; z-index: 2; padding: 6px 12px; border: 0; border-radius: var(--radius-xl); background: var(--accent); color: var(--on-accent); font-size: 12px; font-weight: 700; box-shadow: var(--shadow-1); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 0 8px; display: flex; flex-direction: column; }
  .list:focus { outline: none; } /* focused only from code, as a fallback: no ring */
  .start { padding: 16px 18px; color: var(--text-3); font-size: 13px; }
  .typing { height: 22px; padding: 0 18px; font-size: 12px; color: var(--text-3); }
  .typing strong { color: var(--text); }
  .error { margin: 0 18px 4px; color: var(--danger-text); font-size: 13px; }
  /* a jumped-to message: a fading highlight, or a static outline under reduced motion */
  .list :global(.jump-flash) { animation: jump-flash 2s ease-out; }
  .list :global(.jump-flash.still) { animation: none; outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  @keyframes jump-flash {
    0%, 30% { background: var(--accent-mid); }
    100% { background: transparent; }
  }
  .jump-notice { margin: 0 18px 4px; color: var(--text-3); font-size: 13px; }
</style>
