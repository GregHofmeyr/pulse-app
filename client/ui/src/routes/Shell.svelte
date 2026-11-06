<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { listen } from '@tauri-apps/api/event'
  import ConnText from '../components/ConnText.svelte'
  import Icon from '../components/Icon.svelte'
  import ChannelList from '../components/ChannelList.svelte'
  import MemberList from '../components/MemberList.svelte'
  import MiniPreview from '../components/MiniPreview.svelte'
  import StreamToast from '../components/StreamToast.svelte'
  import CallRingPopover from '../components/CallRingPopover.svelte'
  import VoiceStatus from '../components/VoiceStatus.svelte'
  import VoicePanel from '../components/VoicePanel.svelte'
  import TextChannel from '../components/TextChannel.svelte'
  import ClipsChannel from '../components/ClipsChannel.svelte'
  import Settings, { type Section } from '../components/Settings.svelte'
  import FullProfile from '../components/FullProfile.svelte'
  import NicknameDialog from '../components/NicknameDialog.svelte'
  import ServerMenu from '../components/ServerMenu.svelte'
  import ServerSettings from '../components/ServerSettings.svelte'
  import { requestCreateCategory } from '../lib/categoryUi.svelte'
  import { clickOutside, modal, narrowerThan } from '../lib/actions'
  import { exitFade } from '../lib/exit'
  import { NARROW_BODY, searchMakesRoom, sidebarLayout } from '../lib/layout'
  import { COLUMNS, fitColumns, RIGHT_DEFAULT, RIGHT_MIN } from '../lib/columns'
  import { columns, resetColumn, setColumn } from '../lib/columns.svelte'
  import { CHAT_TOGGLE_ID, setVoiceChat, voiceChat } from '../lib/voiceChat.svelte'
  import ColumnHandle from '../components/ColumnHandle.svelte'
  import ReportBug from '../components/ReportBug.svelte'
  import PatchNotes from '../components/PatchNotes.svelte'
  import { patchNotes } from '../patchNotes'
  import { markPatchNotesSeen, patchNotesSeen } from '../lib/patchNotesSeen.svelte'
  import { installUpdate, restartBlocked, updates } from '../lib/updates.svelte'
  import CrashReportPrompt from '../components/CrashReportPrompt.svelte'
  import ConversationList from '../components/ConversationList.svelte'
  import GoLiveDialog from '../components/GoLiveDialog.svelte'
  import QotyBanner from '../components/QotyBanner.svelte'
  import QotyStart from '../components/QotyStart.svelte'
  import PeopleBoard from '../components/PeopleBoard.svelte'
  import PeoplePicker from '../components/PeoplePicker.svelte'
  import GroupMembers from '../components/GroupMembers.svelte'
  import SearchResults from '../components/SearchResults.svelte'
  import { closeSearch, search } from '../lib/search.svelte'
  // Profiles: the quick card and the full profile
  import ProfileCardPopover from '../components/ProfileCardPopover.svelte'
  import { closeFull, closeRename, openCard, profileUi } from '../lib/profileCard.svelte'
  import { onMount, tick, untrack } from 'svelte'
  import { myServerTips, voice, voiceApi } from '../lib/voice.svelte'
  import { soundboard } from '../lib/soundboard.svelte'
  import { findWatch, followLiveFreeze, stream, streamApi } from '../lib/stream/stream.svelte'
  import { pendingWatchReady } from '../lib/stream/viewer'
  import { callView, openFull } from '../lib/stream/viewer.svelte'
  import { PANEL_ID, PANEL_TOGGLE_ID, setFullPanels } from '../lib/stream/fullPanels'
  import { closeGoLive, goLiveUi } from '../lib/stream/golive.svelte'
  import { api, errorText, type ClipJob } from '../lib/tauri'
  import { app, clock, loadProfile, openDm, ui } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import BadgeToast from '../components/BadgeToast.svelte'
  import GroupIconCrop from '../components/GroupIconCrop.svelte'
  import { badgeStore, logoClick } from '../lib/badges.svelte'
  import { applyEvent, dropClipJob, setClipJob, trimHistory } from '../lib/state'
  import { conversationName, homeBadge, inVoice, serverBadge, watchOpen, type OpenWatch } from '../lib/conversations'
  import { baseName, displayName, isMember, liveIn } from '../lib/selectors'
  import { CONN_PILL_DELAY_MS, isRefusal } from '../lib/conn'
  import { avatarColor, initial } from '../lib/avatar'
  import Avatar from '../components/Avatar.svelte'
  import Name from '../components/Name.svelte'
  import { keybindsApi } from '../lib/keybinds'
  import { watchWindowFocus } from '../lib/windowFocus.svelte'
  import type { User } from '../lib/protocol/User'

  let { user, onLogout }: { user: User; onLogout: () => void } = $props()
  // rings in the voice list: who's talking, plus whose soundboard sound is playing (render only: never voice.speaking)
  const ringing = $derived(soundboard.playing.size ? new Set([...voice.speaking, ...soundboard.playing]) : voice.speaking)
  // Signed in as `user`; app.state.me follows your profile edits (ProfileUpdated) once Ready has landed.
  const me = $derived(app.state.me ?? user)

  const win = getCurrentWindow()
  let activeServerId = $state<string | null>(null)
  let activeChannelId = $state<string | null>(null)
  let settingsOpen = $state(false)
  /** The server a Quote of the Year is being started for (its start dialog is open). */
  let qotyStart = $state<string | null>(null)

  // The animation freeze while you stream; here so it outlives the voice panel. Returning the stop
  // is what ends it with the shell: a `{ followLiveFreeze() }` body would leak it.
  $effect(() => followLiveFreeze())
  let settingsSection = $state<Section>('voice')
  // The reconnecting pill waits CONN_PILL_DELAY_MS, so a blip doesn't flash it; a refusal shows at
  // once (it says why). A JS timer, not a CSS delay, which the .unfocused pause would hold back.
  const connDown = $derived(app.state.conn !== 'connected')
  const connRefused = $derived(isRefusal(app.state.conn))
  let connShown = $state(false)
  $effect(() => {
    if (!connDown) {
      connShown = false
      return
    }
    if (connRefused) {
      connShown = true
      return
    }
    const t = setTimeout(() => (connShown = true), CONN_PILL_DELAY_MS)
    return () => clearTimeout(t)
  })
  // Nothing is bound by default: until they've chosen, nudge people to pick their keys (Windows;
  // on Linux the compositor binds them).
  let pickKeys = $state(false)
  const windows = !navigator.userAgent.includes('Linux')
  async function checkKeybinds() {
    if (!windows) return
    try {
      pickKeys = !(await keybindsApi.get()).chosen
    } catch {
      /* the nudge is optional */
    }
  }
  function openSettings(section: Section = 'voice') {
    settingsSection = section
    settingsOpen = true
  }
  async function dismissPickKeys() {
    pickKeys = false
    try {
      await keybindsApi.set((await keybindsApi.get()).binds) // saving 'none' counts as a choice
    } catch {
      /* shown again next launch */
    }
  }
  let reportOpen = $state(false)
  let notesOpen = $state(false)
  function openPatchNotes() {
    notesOpen = true
    markPatchNotesSeen(patchNotes.id)
  }
  // #82: a newer version, found while running. Never during a call (the core refuses too); never by itself.
  const updateBlocked = $derived(restartBlocked())
  function restartToUpdate() {
    if (updateBlocked || updates.installing) return
    // a failure is logged and shown nowhere else: the button stays, try again later
    installUpdate(true).catch(() => {})
  }
  /** Your avatar and name (#168): one control, opening your own card (status and Settings are on it). */
  let meButton = $state<HTMLButtonElement>()
  const myCardOpen = $derived(!!meButton && profileUi.card?.opener === meButton)
  // Full view takes the whole window: both side panels hide, and a toggle in each top corner of its
  // bar brings one back.
  // Every full view starts with both hidden (nothing is remembered); swapping its stream or coming
  // back to it from the mini-preview is the same full view, so what you opened stays open.
  const panels = $state({ left: false, right: false })
  const fullShown = $derived(callView.fullViews > 0)
  let hadFull = false
  $effect(() => {
    const has = !!callView.full
    untrack(() => {
      if (has && !hadFull) {
        panels.left = false
        panels.right = false
      }
      hadFull = has
    })
  })
  const hideLeft = $derived(fullShown && !panels.left)
  // A narrow window shows one side panel: a replies panel opened from search hides the results meanwhile (#97).
  let narrowBody = $state(false)
  // search results kept behind a voice channel's chat (#230) don't show meanwhile
  const searchShown = $derived(search.open && !search.behindChat)
  const searchAside = $derived(searchMakesRoom({ narrow: narrowBody, searchOpen: searchShown, threadOpen: !!ui.openThreadId }))
  const hideRight = $derived((fullShown && !panels.right) || searchAside)
  let sidePanel = $state<HTMLElement>()
  let rightPanel = $state<HTMLElement>()
  // Before a panel hides: a menu open in it closes (an unseen menu would take Esc first), and focus in
  // it moves to the toggle that brings it back (or it would fall to the top of the page).
  $effect.pre(() => {
    const [left, right] = [hideLeft, hideRight]
    untrack(() => {
      if (left) serverMenuOpen = false
      const at = document.activeElement
      const to = left && sidePanel?.contains(at) ? 'left' : right && rightPanel?.contains(at) ? 'right' : null
      if (to) void tick().then(() => document.getElementById(PANEL_TOGGLE_ID[to])?.focus())
    })
  })
  const leftName = $derived(activeServerId ? 'Channels' : 'Conversations')
  setFullPanels({
    get left() { return panels.left },
    get right() { return panels.right },
    get leftName() { return leftName },
    get hasRight() { return !!(search.open || activeServerId || activeChannel?.kind === 'group') },
    get rightName() { return rightKind === 'chat' ? 'Chat' : 'Members' },
    toggle: (side) => (panels[side] = !panels[side]),
  })
  let serverMenuOpen = $state(false)
  let serverToggle = $state<HTMLButtonElement>()
  /** The same for the server menu. */
  function closeServerMenu(e: Event) {
    serverMenuOpen = false
    if (e instanceof KeyboardEvent) serverToggle?.focus()
  }
  // The server whose settings are open: the active one (sidebar header menu), or any tab's (its right-click menu, #161).
  let serverSettingsId = $state<string | null>(null)
  const settingsServer = $derived(serverSettingsId ? (app.state.servers.find((s) => s.id === serverSettingsId) ?? null) : null)
  // Both belong to the server they were opened on.
  $effect(() => {
    void activeServerId
    serverMenuOpen = false
    serverSettingsId = null
  })
  let creating = $state(false)
  let newName = $state('')
  let createError = $state('')
  /** People picker: a new DM/group from Home, or Add people in a conversation. */
  let picker = $state<null | { mode: 'new' } | { mode: 'add'; channelId: string }>(null)
  // Badges depend on mute expiry: they re-evaluate on the shared clock.
  const nowIso = $derived(clock.nowIso)
  onMount(() => void checkKeybinds())
  onMount(watchWindowFocus)
  // The clips you're sending: the ones the core already has (after a restart, the ones left over), then every
  // change as `clip://job`. An event beats the list for its job (it's newer). A posted clip's card stays a
  // moment (the clip itself may arrive just after), then goes even if the clip never showed up here.
  const POSTED_CARD_MS = 5_000
  onMount(() => {
    let off: (() => void) | null = null
    let gone = false
    const timers = new Set<ReturnType<typeof setTimeout>>()
    void (async () => {
      const heard = new Set<string>()
      try {
        const f = await listen<ClipJob>('clip://job', (e) => {
          const job = e.payload
          heard.add(job.id)
          app.state = setClipJob(app.state, job)
          if (job.stage !== 'done' || !job.message_id) return
          const t = setTimeout(() => {
            timers.delete(t)
            app.state = dropClipJob(app.state, job.id)
          }, POSTED_CARD_MS)
          timers.add(t)
        })
        if (gone) return f()
        off = f
      } catch {
        // no progress events: the jobs still show as they were at start
      }
      const jobs = await api.clipJobs().catch(() => [] as ClipJob[])
      if (gone) return
      for (const j of jobs ?? []) if (!heard.has(j.id)) app.state = setClipJob(app.state, j)
    })()
    return () => {
      gone = true
      off?.()
      for (const t of timers) clearTimeout(t)
    }
  })
  // The channel whose messages are on screen (the alert rule and jumps go by it): a voice channel's only while its
  // chat panel shows (#226); its stage alone shows none.
  $effect(() => {
    ui.openChannelId = activeChannel?.kind === 'voice' ? (chatOnScreen ? activeChannelId : null) : activeChannelId
  })
  // Leaving a channel trims it to its newest messages (scrolling up there fetches older ones again).
  // Done when the next channel opens, not in a teardown: a teardown on the shell going away (sign-out)
  // is handed the state from before the reset, and writing that back restores the old session.
  let shown: string | null = null
  $effect(() => {
    const left = shown
    shown = activeChannelId
    if (left && left !== shown) untrack(() => (app.state = trimHistory(app.state, left)))
  })
  const home = $derived(homeBadge(app.state, nowIso))
  /** Where the call you're in lives (#220): its server's id, 'home' for a DM or group call, or null. */
  const callAt = $derived.by(() => {
    const ch = voice.channelId ? app.state.channels[voice.channelId] : undefined
    return !ch ? null : (ch.server_id ?? 'home')
  })

  // Right-click a server tab (#161): its settings, and mute/unmute the whole server (ServerMenu).
  let serverMenu = $state<null | { id: string; x: number; y: number; opener: HTMLElement }>(null)

  // Removed from (or left) the open conversation: step back to Home and say why. The open server
  // channel was deleted: go to another of the server's channels. Only fires when a channel we had
  // actually seen disappears (a just-created one may not have arrived yet).
  let notice = $state('')
  let watch: OpenWatch | null = null
  let lastTitle = ''
  $effect(() => {
    const id = activeChannelId
    if (!id) {
      watch = null
      return
    }
    const ch = app.state.channels[id]
    if (ch) lastTitle = ch.server_id && ch.kind !== 'voice' ? `#${ch.name}` : conversationName(app.state, id)
    const w = watchOpen(watch, id, !!ch)
    watch = w.state
    if (!w.removed) return
    const server = activeServerId
    if (server) untrack(() => selectServer(server))
    else activeChannelId = null
    notice = server ? `${lastTitle} was deleted.` : `You're no longer in ${lastTitle}.`
    setTimeout(() => (notice = ''), 4000)
  })
  const pickerTarget = $derived(picker?.mode === 'add' ? app.state.channels[picker.channelId] : null)
  const pickerOthers = $derived(
    picker?.mode === 'add' ? (app.state.dmMembers[picker.channelId] ?? []).filter((u) => u !== app.state.me?.id) : [],
  )

  async function openConversation(id: string) {
    activeServerId = null
    activeChannelId = id
  }

  /** Put a just-created conversation into state before showing it (its gateway event may lag). */
  function adopt(ch: import('../lib/protocol/Channel').Channel): string {
    if (!app.state.channels[ch.id]) app.state = applyEvent(app.state, { t: 'ChannelCreated', d: { channel: ch } }, Date.now())
    return ch.id
  }

  async function messageUser(userId: string) {
    try {
      await openConversation(await openDm(userId))
    } catch (e) {
      error = errorText(e)
    }
  }

  async function pickerDone(ids: string[]) {
    const p = picker
    picker = null
    if (!p) return
    try {
      if (p.mode === 'new') {
        await openConversation(adopt(await api.createDm(ids)))
      } else {
        const ch = app.state.channels[p.channelId]
        const members = (app.state.dmMembers[p.channelId] ?? []).filter((u) => u !== app.state.me?.id)
        if (ch?.kind === 'dm') {
          // Discord-style: adding people to a DM starts a new group; the DM stays as it is.
          await openConversation(adopt(await api.createDm([...members, ...ids])))
        } else {
          await api.addMembers(p.channelId, ids)
        }
      }
    } catch (e) {
      error = errorText(e)
    }
  }

  async function createServer(e: SubmitEvent) {
    e.preventDefault()
    createError = ''
    try {
      const s = await api.createServer(newName.trim())
      creating = false
      newName = ''
      // the ServerCreated/ChannelCreated events may still be in flight: select once they land
      const pick = () => (app.state.servers.some((x) => x.id === s.id) ? selectServer(s.id) : setTimeout(pick, 50))
      pick()
    } catch (err) {
      createError = errorText(err)
    }
  }
  let error = $state('')

  const servers = $derived(app.state.servers)
  const activeServer = $derived(servers.find((s) => s.id === activeServerId) ?? null)
  const member = $derived(activeServerId ? isMember(app.state, activeServerId) : false)
  const activeChannel = $derived(activeChannelId ? app.state.channels[activeChannelId] : null)
  // Resizable side columns (#160). What the right column holds sets its default; a column that isn't showing (full
  // view, the narrow-window search hand-off, Home with nothing on the right) has no handle and takes no room. Saved
  // widths are fitted to the body's width, so a shrinking window narrows them (lib/columns.ts).
  // A voice channel's chat (#226) takes the members list's place beside its stage while it's open, with its own width.
  const chatOpen = $derived(!!activeChannelId && activeChannel?.kind === 'voice' && !!activeServerId && member && voiceChat.open)
  // the results kept behind a voice channel's chat (#230) come back once it's gone (closed, or another channel)
  $effect(() => {
    if (search.behindChat && !chatOpen) search.behindChat = false
  })
  const rightKind = $derived<keyof typeof RIGHT_DEFAULT | null>(
    searchShown ? 'search' : chatOpen ? 'chat' : activeServerId ? 'members' : activeChannel?.kind === 'group' ? 'group' : null)
  const showLeft = $derived(!hideLeft)
  const showRight = $derived(!hideRight && rightKind !== null)
  const chatOnScreen = $derived(showRight && rightKind === 'chat')
  const rightColumn = $derived(rightKind === 'chat' ? 'chat' : 'right')
  const rightSaved = $derived(columns[rightColumn])
  // A click's ask for the chat's message box counts only while the chat can show: one made while it can't (search
  // results in its place, full view) mustn't take the focus whenever it next appears.
  $effect(() => {
    if (!chatOnScreen && voiceChat.focusWanted > untrack(() => voiceChat.focusDone)) voiceChat.focusDone = voiceChat.focusWanted
  })
  let bodyEl = $state<HTMLElement>()
  let bodyWidth = $state(0)
  $effect(() => {
    if (!bodyEl || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver((entries) => (bodyWidth = entries[entries.length - 1]?.contentRect.width ?? 0))
    ro.observe(bodyEl)
    return () => ro.disconnect()
  })
  const rightMin = $derived(RIGHT_MIN[rightKind ?? 'members'])
  const fitted = $derived(fitColumns({ saved: { left: columns.left, right: rightSaved }, body: bodyWidth, left: showLeft, right: showRight, rightDefault: RIGHT_DEFAULT[rightKind ?? 'members'], rightMin }))
  const rightLabel = $derived(rightKind === 'search' ? 'Resize the search results' : rightKind === 'chat' ? 'Resize the chat' : 'Resize the members list')
  /** The chat's close button: the panel goes, and the focus it had goes to the stage's toggle. */
  function closeChat() {
    const at = document.activeElement
    const inside = !at || at === document.body || !!rightPanel?.contains(at)
    setVoiceChat(false)
    if (inside) void tick().then(() => document.getElementById(CHAT_TOGGLE_ID)?.focus())
  }
  // The channel views below take their id from activeChannelId (a plain string), never activeChannel.id: for the
  // moment a channel leaves the data before its view goes (a sign-out's reset, a delete), activeChannel is undefined,
  // and reading .id there threw and left the window blank after a sign-in (2026-11-03).

  /** Show channel `id` (with its server), if it exists. */
  function showChannel(id: string) {
    const ch = app.state.channels[id]
    if (!ch) return
    activeServerId = ch.server_id
    activeChannelId = id
  }
  // The call view asked to be shown (a Watch from a toast or popover, Join & watch, the mini-preview).
  $effect(() => {
    const id = callView.goTo
    if (!id) return
    untrack(() => {
      showChannel(id)
      callView.goTo = null
    })
  })
  // Your own profile, once per connection (Ready empties the cache): its unseen badges light the
  // [NEW] dot on your avatar even when they were given while you were away (#49).
  const ownCached = $derived(!!app.state.profiles[me.id])
  $effect(() => {
    if (app.state.conn !== 'connected' || ownCached) return
    untrack(() => loadProfile(me.id).catch(() => {}))
  })
  // Edit profile on your own card asked for Settings → Profile.
  $effect(() => {
    if (!profileUi.edit) return
    untrack(() => {
      profileUi.edit = false
      openSettings('profile')
    })
  })
  // Settings on your own card (#168).
  $effect(() => {
    if (!profileUi.settings) return
    untrack(() => {
      profileUi.settings = false
      openSettings()
    })
  })
  // Search covers one server, or one conversation: going elsewhere closes its results (a result's
  // click stays in the same scope, so it never does). Signing out closes them too.
  $effect(() => {
    const sc = search.scope
    if (!search.open || !sc) return
    const stays = sc.kind === 'server' ? activeServerId === sc.serverId : activeServerId === null && activeChannelId === sc.channelId
    if (!stays) untrack(closeSearch)
  })
  $effect(() => () => untrack(closeSearch))
  // Something asked for a line to show for a moment (a jump into a deleted channel).
  $effect(() => {
    const line = ui.notice
    if (!line) return
    untrack(() => {
      notice = line
      ui.notice = null
      setTimeout(() => (notice = ''), 4000)
    })
  })
  // A jump to a message (lib/jump.ts) asked for its channel.
  $effect(() => {
    const id = ui.goTo
    if (!id) return
    untrack(() => {
      showChannel(id)
      // a jump or a notification into a voice channel's chat (#226): its panel opens to show it, in the right column
      // search results would otherwise hold; a search hit there keeps them behind the chat, to come back to (#230)
      if (app.state.channels[id]?.kind === 'voice') {
        if (search.open) search.behindChat = true
        setVoiceChat(true)
      }
      ui.goTo = null
    })
  })

  // The watch in full view ended (stopped, the stream ended, voice left): no full view to go back to.
  $effect(() => {
    const f = callView.full
    if (f && !findWatch(f.channelId, f.streamer)) callView.full = null
  })

  // Join & watch: watch once the join has connected in that channel. A failed join, a join elsewhere
  // meanwhile, or the stream ending first drops it.
  $effect(() => {
    const p = callView.joinThenWatch
    if (!p) return
    const v = { channelId: voice.channelId, connection: voice.connection }
    const live = !!liveIn(app.state, p.channelId)[p.streamer]
    const stillJoining = voice.joining === p.channelId || v.channelId === p.channelId
    if (!live || !stillJoining || voice.joinFailed) {
      callView.joinThenWatch = null
      return
    }
    if (!pendingWatchReady(p, v)) return
    untrack(() => {
      callView.joinThenWatch = null
      void streamApi.watch(p.channelId, p.streamer)
      openFull(p)
    })
  })

  // Leaving voice (or losing the connection) while the Go Live dialog is open: drop the picked capture.
  $effect(() => {
    const p = goLiveUi.picked
    const here = voice.channelId === goLiveUi.channelId && voice.connection === 'connected'
    if (!here && (p || goLiveUi.channelId)) {
      // a picker error or capture from a call you've left must not come back later
      p?.track.stop()
      closeGoLive()
    }
  })

  /** The voice channel the main area shows, if any: its watches show in the grid, the rest in the mini-preview. */
  const shownCall = $derived(activeChannel?.kind === 'voice' && (!activeServerId || member) ? activeChannel.id : null)
  const mini = $derived.by(() => {
    const w = stream.watching.find((x) => x.channelId !== shownCall)
    return w ? { channelId: w.channelId, streamer: w.streamer, key: `${w.channelId}/${w.streamer}` } : null
  })

  // Launch lands on Home (a clean slate); only the person who creates a server is moved into it.

  function selectServer(id: string) {
    activeServerId = id
    // the top text channel, as the sidebar lists them (positions count within a category, #54)
    const l = sidebarLayout(app.state, id)
    const general = [...l.top, ...l.sections.flatMap((x) => x.channels)].find((c) => c.kind === 'text')
    activeChannelId = general?.id ?? null
  }

  async function join() {
    if (!activeServerId) return
    try {
      await api.joinServer(activeServerId)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function logout() {
    await api.logout()
    onLogout()
  }
</script>

{#snippet callBadge()}<span class="call-badge" aria-hidden="true"><Icon name="speaker" size={10} /></span>{/snippet}

<div class="app">
  <header data-tauri-drag-region>
    <!-- the logo egg (#49): 7 quick clicks; hidden on purpose, so no button or keyboard path -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="logo" aria-hidden="true" onclick={() => logoClick()}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
    </span>
    <nav aria-label="Servers">
      <button class="tab" class:active={activeServerId === null} aria-label={callAt === 'home' ? "Home (you're in a call here)" : undefined} onclick={() => { activeServerId = null; activeChannelId = null }}>
        {#if callAt === 'home'}<span class="call-here call-home"><Icon name="home" size={16} />{@render callBadge()}</span>{:else}<Icon name="home" size={16} />{/if} Home{#if home > 0}<span class="pill">{home > 99 ? '99+' : home}</span>{/if}
      </button>
      <span class="sep"></span>
      {#each servers as s (s.id)}
        {@const badge = serverBadge(app.state, s.id, nowIso)}
        {@const talking = inVoice(app.state, s.id)}
        <button class="tab" class:active={activeServerId === s.id} aria-label={callAt === s.id ? `${s.name} (you're in a call here)` : undefined} onclick={() => selectServer(s.id)}
          oncontextmenu={(e) => { e.preventDefault(); serverMenu = { id: s.id, x: e.clientX, y: e.clientY, opener: e.currentTarget } }}>
          {#if callAt === s.id}
            <span class="chip call-here" style:background={avatarColor(s.id)}>{initial(s.name)}{@render callBadge()}</span>{s.name}
          {:else}
            <span class="chip" style:background={avatarColor(s.id)}>{initial(s.name)}</span>{s.name}
          {/if}
          <!-- the badge already says it for your own call's server -->
          {#if talking > 0 && callAt !== s.id}<span class="invc" aria-label="{talking} in voice" data-tip="{talking} in voice"><Icon name="speaker" size={14} /></span>{/if}
          {#if badge.mentions > 0}<span class="pill">{badge.mentions}</span>{:else if badge.dot}<span class="dot" aria-label="new messages"></span>{/if}
        </button>
      {/each}
      <button class="add" aria-label="Create a server" onclick={() => (creating = true)}><Icon name="plus" size={16} /></button>
    </nav>
    <div class="grow" data-tauri-drag-region></div>
    {#if connDown && connShown}
      <span class="conn" role="status"><ConnText /></span>
    {/if}
    {#if pickKeys}
      <span class="conn auto-report" role="status">Pick your mute and deafen keys. <button class="link" onclick={() => openSettings('keybinds')}>Choose</button> <button aria-label="Dismiss" onclick={dismissPickKeys}>×</button></span>
    {/if}
    <CrashReportPrompt />
    {#if ui.autoReported}
      <span class="conn auto-report" role="status">Something went wrong in Pulse last time, so we sent Greg a report. <button aria-label="Dismiss" onclick={() => (ui.autoReported = false)}>×</button></span>
    {/if}
    <!-- #155: icon only; the app-wide tooltip reads its aria-label -->
    <!-- #82: a newer Pulse is ready; aria-disabled, not disabled, in a call: a disabled button swallows the hover that shows the tooltip -->
    {#if updates.offer}
      <button class="win update" class:off={updateBlocked} aria-disabled={updateBlocked} data-tip={updateBlocked ? 'Finish your call first' : undefined} onclick={restartToUpdate}>Restart to update</button>
    {/if}
    <!-- #207: what's new; a dot until this release's notes are opened once on this device -->
    <button class="win notes" aria-label="Patch notes" onclick={openPatchNotes}><Icon name="scroll" size={16} />{#if patchNotesSeen.id !== patchNotes.id}<span class="new-dot" aria-hidden="true"></span>{/if}</button>
    <button class="win report" aria-label="Report a bug or suggest an idea" onclick={() => (reportOpen = true)}><Icon name="bug" size={16} /></button>
    <button class="win" aria-label="Minimise" onclick={() => win.minimize()}><Icon name="min" size={14} /></button>
    <button class="win" aria-label="Maximise" onclick={() => win.toggleMaximize()}><Icon name="square" size={14} /></button>
    <button class="win" aria-label="Close" onclick={() => win.close()}><Icon name="x" size={14} /></button>
  </header>

  <div class="body" bind:this={bodyEl} use:narrowerThan={{ px: NARROW_BODY, set: (v) => (narrowBody = v) }}>
    <aside id={PANEL_ID.left} class="panel side" hidden={hideLeft} bind:this={sidePanel} style:width="{fitted.left}px">
      <div class="panel-head">
        {#if activeServer}
          <button bind:this={serverToggle} class="head-btn" data-menu-toggle aria-haspopup="menu" aria-expanded={serverMenuOpen}
            onclick={() => (serverMenuOpen = !serverMenuOpen)}>{activeServer.name}<Icon name="chevron" size={14} /></button>
          {#if serverMenuOpen}
            <div class="menu head-menu menu-surface pop-in" role="menu" use:clickOutside={closeServerMenu}>
              <button class="menu-item" role="menuitem" onclick={() => { serverMenuOpen = false; serverSettingsId = activeServerId }}>Server settings</button>
              {#if member && activeServerId && app.state.layouts[activeServerId]}
                <button class="menu-item" role="menuitem" onclick={() => { serverMenuOpen = false; if (activeServerId) requestCreateCategory(activeServerId) }}>Create category</button>
              {/if}
            </div>
          {/if}
        {:else}Direct messages{/if}
      </div>
      {#if activeServerId}
        <ChannelList serverId={activeServerId} {activeChannelId} voiceChannelId={voice.channelId} speaking={ringing}
          onSelect={(id) => (activeChannelId = id)}
          onJoinVoice={(id) => { activeChannelId = id; if (voice.channelId !== id && member) void voiceApi.join(id) }}
          onOpenChat={(id) => { activeChannelId = id; setVoiceChat(true, true) }}
          onMessage={messageUser} onVoiceSettings={() => openSettings('voice')} />
      {:else}
        <ConversationList activeId={activeChannelId} onOpen={openConversation} onNew={() => (picker = { mode: 'new' })}
          onLeft={(id) => { if (activeChannelId === id) activeChannelId = null }} />
      {/if}
      <VoiceStatus />
      <div class="me">
        <!-- your avatar and name: one click opens your own card, where your status and Settings are (#168); the same
             click again closes it (openCard toggles). Log out is at the bottom of Settings. -->
        <button bind:this={meButton} type="button" class="who-btn" aria-label={badgeStore.unseen > 0 ? 'Your profile (new badges)' : 'Your profile'} data-tip="Your profile"
          aria-haspopup="dialog" aria-expanded={myCardOpen} onclick={(e) => openCard(me.id, null, e.currentTarget)}>
          <span class="me-avatar"><Avatar id={me.id} name={baseName(me)} size={34} presence={presenceOf(app.state, me.id, clock.nowIso)} />{#if badgeStore.unseen > 0}<span class="new-dot" aria-hidden="true"></span>{/if}</span>
          <span class="who"><Name userId={me.id} serverId={null} text={baseName(me)} interactive={false} surface="side" /><small>{app.state.conn === 'connected' ? presenceOf(app.state, me.id, clock.nowIso).label : 'Offline'}</small></span>
        </button>
        {#if voice.channelId}
          {@const tips = myServerTips()}
          <button class="icon-btn me-btn" class:on={voice.controls.muted} class:server={!!tips.mute} data-tip={tips.mute ?? undefined} aria-pressed={voice.controls.muted} aria-label={(voice.controls.muted ? "Unmute" : "Mute") + (tips.mute ? ` (${tips.mute})` : "")} onclick={() => voiceApi.toggleMute()}>
            <Icon name={voice.controls.muted || tips.mute ? 'micOff' : 'mic'} />
          </button>
          <button class="icon-btn me-btn" class:on={voice.controls.deafened} class:server={!!tips.deafen} data-tip={tips.deafen ?? undefined} aria-pressed={voice.controls.deafened} aria-label={(voice.controls.deafened ? "Undeafen" : "Deafen") + (tips.deafen ? ` (${tips.deafen})` : "")} onclick={() => voiceApi.toggleDeafen()}>
            <Icon name={voice.controls.deafened || tips.deafen ? 'headphonesOff' : 'headphones'} />
          </button>
        {/if}
        <button class="icon-btn me-btn" aria-label="Settings" onclick={() => openSettings()}><Icon name="gear" /></button>
      </div>
    </aside>
    {#if showLeft}
      <ColumnHandle side="left" width={fitted.left} min={COLUMNS.left.min} max={fitted.maxLeft} controls={PANEL_ID.left}
        label="Resize the channel list" onResize={(w, done) => setColumn('left', w, done)} onReset={() => resetColumn('left')} />
    {/if}

    <main class="panel content">
      {#if activeServerId && member && !fullShown}<QotyBanner serverId={activeServerId} />{/if}
      {#if activeServerId && !member}
        <div class="join">
          <p>You're not in <strong>{activeServer?.name}</strong> yet.</p>
          <button class="btn btn-lg btn-primary" onclick={join}>Join server</button>
          {#if error}<p class="error">{error}</p>{/if}
        </div>
      {:else if activeChannel?.kind === 'voice' && activeChannelId}
        <VoicePanel channelId={activeChannelId} serverId={activeServerId} onMessage={messageUser} onVoiceSettings={() => openSettings('voice')} />
      {:else if activeChannel?.kind === 'clips' && activeServerId && activeChannelId}
        {#key activeChannelId}<ClipsChannel channelId={activeChannelId} serverId={activeServerId} />{/key}
      {:else if activeChannel && activeChannelId}
        {#key activeChannelId}<TextChannel channelId={activeChannelId} serverId={activeServerId}
          onAddPeople={() => activeChannelId && (picker = { mode: 'add', channelId: activeChannelId })}
          onLeft={() => (activeChannelId = null)} onStartQoty={() => (qotyStart = activeServerId)} onVoiceSettings={() => openSettings('voice')} />{/key}
      {:else if activeServerId === null}
        <PeopleBoard onOpen={openConversation} onJoinVoice={(ch, srv) => { selectServer(srv); activeChannelId = ch; if (voice.channelId !== ch) void voiceApi.join(ch) }} />
      {:else}
        <p class="empty">Pick a channel.</p>
      {/if}
      {#each mini ? [mini] : [] as m (m.key)}<MiniPreview channelId={m.channelId} streamer={m.streamer} />{/each}
    </main>

    {#if showRight}
      <ColumnHandle side="right" width={fitted.right} min={rightMin} max={fitted.maxRight} controls={PANEL_ID.right}
        label={rightLabel} onResize={(w, done) => setColumn(rightColumn, w, done)} onReset={() => resetColumn(rightColumn)} />
    {/if}
    <!-- one line: nothing but the list inside, so an empty wrapper (Home) matches :empty and takes no gap. A saved
         width reaches the list as --right-w; unsaved, each list keeps its own default. -->
    <div id={PANEL_ID.right} class="right-panel" hidden={hideRight} bind:this={rightPanel} style:--right-w={rightSaved === null ? null : `${fitted.right}px`}>{#if searchShown}<SearchResults />{:else if chatOpen && activeChannelId}<aside class="voice-chat fade-in" aria-label="Chat" onanimationend={(e) => e.currentTarget.classList.remove('fade-in')} out:exitFade onoutrostart={(e) => e.currentTarget.classList.add('leaving')}>{#key activeChannelId}<TextChannel channelId={activeChannelId} serverId={activeServerId} panel={{ onClose: closeChat, shown: chatOnScreen }} />{/key}</aside>{:else if activeServerId}<MemberList serverId={activeServerId} onMessage={messageUser} />{:else if activeChannel?.kind === 'group' && activeChannelId}<GroupMembers channelId={activeChannelId} />{/if}</div>
  </div>
</div>
<StreamToast />
<CallRingPopover />
<BadgeToast />
<GroupIconCrop />
{#if settingsServer}<ServerSettings server={settingsServer} onClose={() => (serverSettingsId = null)} />{/if}
{#if settingsOpen}<Settings section={settingsSection} onLogout={logout} onClose={() => { settingsOpen = false; void checkKeybinds() }} />{/if}
{#if goLiveUi.picked && goLiveUi.channelId}<GoLiveDialog channelId={goLiveUi.channelId} bind:picked={goLiveUi.picked} onClose={closeGoLive} />{/if}
{#if qotyStart}<QotyStart serverId={qotyStart} onClose={() => (qotyStart = null)} />{/if}
{#if reportOpen}<ReportBug onClose={() => (reportOpen = false)} />{/if}
{#if notesOpen}<PatchNotes notes={patchNotes} onClose={() => (notesOpen = false)} />{/if}
<!-- Profiles: the quick card (one at a time), and the full profile's place (Task 12 renders it here) -->
<ProfileCardPopover />
{#if profileUi.full}<FullProfile userId={profileUi.full.userId} serverId={profileUi.full.serverId} onClose={closeFull} />{/if}
{#if profileUi.rename}<NicknameDialog target={profileUi.rename} onClose={closeRename} />{/if}
{#if creating}
  <div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={() => (creating = false)}></div>
  <div class="modal dialog pop-in" role="dialog" aria-modal="true" aria-label="Create a server" tabindex="-1" use:modal={() => (creating = false)}><form class="dialog-form" onsubmit={createServer}>
    <h2 class="modal-title">Create a server</h2>
    <label>NAME <input class="input" bind:value={newName} maxlength="64" required placeholder="Game Night" /></label>
    {#if createError}<p class="error">{createError}</p>{/if}
    <div class="dialog-actions">
      <button type="button" class="btn btn-lg btn-secondary" onclick={() => (creating = false)}>Cancel</button>
      <button class="btn btn-lg btn-primary" disabled={!newName.trim()}>Create</button>
    </div>
  </form></div>
  </div>
{/if}

{#if picker?.mode === 'new'}
  <PeoplePicker title="New message" hint="Pick one person for a DM, or several for a group." action="Start" onDone={pickerDone} onClose={() => (picker = null)} />
{:else if picker?.mode === 'add' && pickerTarget?.kind === 'dm'}
  {@const other = app.state.people[pickerOthers[0]] ? displayName(app.state, null, pickerOthers[0]) : 'them'}
  <PeoplePicker title="Add people" fixed={pickerOthers} action="Create group"
    hint="This starts a new group with you, {other} and the people you pick. Your DM with {other} stays as it is."
    onDone={pickerDone} onClose={() => (picker = null)} />
{:else if picker?.mode === 'add'}
  <PeoplePicker title="Add people" fixed={pickerOthers} hint="They'll see the group's history." action="Add"
    onDone={pickerDone} onClose={() => (picker = null)} />
{/if}
{#if notice}<div class="toast notice fade-in" role="status" out:exitFade>{notice}</div>{/if}
{#if serverMenu}
  {@const m = serverMenu}
  {@const srv = app.state.servers.find((x) => x.id === m.id)}
  {#if srv}
    {#key m}
      <ServerMenu serverId={m.id} name={srv.name} x={m.x} y={m.y} opener={m.opener} onClose={() => (serverMenu = null)}
        onSettings={() => (serverSettingsId = m.id)} onError={(msg) => (error = msg)}
        onCreateCategory={() => { if (activeServerId !== m.id) selectServer(m.id); requestCreateCategory(m.id) }} />
    {/key}
  {/if}
{/if}

<style>
  /* the dialog box is the div (role=dialog); the form inside lays out as its children */
  .dialog-form { display: contents; }
  .app { height: 100%; display: flex; flex-direction: column; background: var(--bg-0); }
  header { height: 52px; flex-shrink: 0; display: flex; align-items: center; gap: 6px; padding: 0 10px 0 12px; --call-gap: var(--bg-0); }
  /* Home's icon in your call's ring (#220): a little room inside, like a server's chip */
  .tab .call-home { padding: 3px; margin: -3px; }
  .tab:hover { --call-gap: var(--bg-2); }
  .tab.active { --call-gap: var(--bg-3); }
  .logo { width: 30px; height: 30px; border-radius: var(--radius-sm); background: var(--accent); color: var(--on-accent); display: grid; place-items: center; margin-right: 8px; }
  nav { display: flex; align-items: center; gap: 6px; min-width: 0; overflow: hidden; }
  .tab { height: 36px; padding: 0 12px 0 10px; border-radius: var(--radius-md); border: 1px solid transparent; background: transparent; color: var(--text-2); display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; white-space: nowrap; }
  .tab:hover { background: var(--bg-2); }
  /* background and colour only: a bolder weight would widen the tab and nudge the ones after it */
  .tab.active { background: var(--bg-3); border-color: var(--bg-4); color: var(--text); }
  .chip { width: 22px; height: 22px; border-radius: var(--radius-xs); color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 4px; }
  .grow { flex: 1; align-self: stretch; }
  .conn { font-size: 12px; font-weight: 600; color: var(--warn-text); padding: 4px 10px; border-radius: var(--radius-lg); background: var(--warn-soft); margin-right: 6px; }
  .win { width: 32px; height: 28px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .win:hover { background: var(--bg-2); color: var(--text); }
  /* Report a bug (#155): the warn colour it had in the account menu, on hover */
  .win.report:hover, .win.report:focus-visible { color: var(--warn-text); }
  /* Patch notes (#207): the unseen dot sits on the icon's corner, cut out of the title bar (or its hover pill) */
  .win.notes { position: relative; --ring: var(--bg-0); }
  .win.notes:hover { --ring: var(--bg-2); }
  .win.notes .new-dot { top: 3px; right: 6px; width: 8px; height: 8px; }
  /* Restart to update (#82): the same quiet title bar button, with words; dimmed (no hover) while in a call */
  .win.update { width: auto; padding: 0 10px; font-size: 12px; font-weight: 600; white-space: nowrap; }
  .win.update.off { opacity: .5; cursor: not-allowed; }
  .win.update.off:hover { background: transparent; color: var(--text-3); }
  .body { flex: 1; min-height: 0; display: flex; gap: 8px; padding: 0 8px 8px; }
  .panel { border-radius: var(--radius); overflow: hidden; }
  .side { width: 264px; flex-shrink: 0; background: var(--bg-1); display: flex; flex-direction: column; }
  /* hidden, not removed: what the panels hold (an open menu, a draft) survives full view */
  .side[hidden] { display: none; }
  /* a real box (aria-controls points at it; a display:contents one leaves the accessibility tree) */
  .right-panel { position: relative; display: flex; flex-shrink: 0; min-height: 0; }
  .right-panel[hidden], .right-panel:empty { display: none; }
  /* a voice channel's chat (#226): a right column like the search results, holding the channel view; it fades in
     (.fade-in, dropped once done so its will-change doesn't hold a layer: not under reduced motion, while streaming or
     unfocused) and fades out (exitFade, under the same rules, #230) laid over the column, so what takes its place
     (the members list, the search results) is already there underneath, at its own width */
  .voice-chat { width: var(--right-w, 360px); flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; border-radius: var(--radius); background: var(--bg-1); overflow: hidden; }
  .voice-chat:global(.leaving) { position: absolute; top: 0; right: 0; bottom: 0; left: 0; z-index: 1; width: auto; pointer-events: none; }
  .panel-head { position: relative; height: 52px; flex-shrink: 0; padding: 0 16px; display: flex; align-items: center; font-size: 15px; font-weight: 600; border-bottom: 1px solid var(--line-soft); }
  /* the hover is a rounded pill inside the header, with the text where the padding put it */
  .head-btn { flex: 1; min-width: 0; height: 40px; margin: 0 -8px; padding: 0 8px; border-radius: var(--radius-md); display: flex; align-items: center; justify-content: space-between; gap: 8px; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; }
  .head-btn:hover { background: var(--bg-2); }
  /* two classes: .menu (declared below) must not win `bottom` back */
  .menu.head-menu { left: 8px; right: 8px; top: calc(100% + 4px); bottom: auto; --pop-origin: top; }
  .me { padding: 10px 10px 10px 12px; background: var(--bg-inset); display: flex; align-items: center; gap: 10px; }
  .who { flex: 1; display: flex; flex-direction: column; font-size: 13px; font-weight: 600; }
  .who small { font-size: 12px; font-weight: 400; color: var(--text-3); }
  .add { width: 36px; height: 36px; flex-shrink: 0; border: 1px dashed var(--bg-4); border-radius: var(--radius-md); background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .add:hover { color: var(--text); border-color: var(--text-3); }
  .me { position: relative; }
  .me-avatar { position: relative; flex-shrink: 0; display: flex; border-radius: 50%; }
  /* unseen badges (#49): a small dot until you open your trophy case */
  .new-dot { position: absolute; top: -1px; right: -1px; width: 10px; height: 10px; border-radius: 50%; background: var(--badge-new); box-shadow: 0 0 0 2px var(--ring); }
  /* the avatar sits in the button's hover pill with the name (#168); the negative margin keeps it where it was */
  .who-btn { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; margin-left: -6px; padding: 4px 6px; border: 0; border-radius: var(--radius-md); background: transparent; color: inherit; text-align: left; }
  .who-btn:hover { background: var(--bg-2); }
  .who-btn .who { overflow: hidden; }
  .menu { position: absolute; left: 8px; bottom: calc(100% + 6px); z-index: 10; min-width: 200px; }
  /* not the bold of the header it can sit under */
  .menu button { font-weight: 400; }
  .auto-report button { border: 0; background: none; color: inherit; font-size: 15px; line-height: 1; padding: 0 0 0 6px; }
  .auto-report button.link { font-size: inherit; font-weight: 600; text-decoration: underline; padding: 0 2px; }
  .overlay { position: fixed; inset: 0; z-index: 20; }
  .dialog { width: min(420px, calc(100vw - 32px)); gap: 14px; }
  .dialog label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  .dialog-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .me-btn { color: var(--text-2); }
  .me-btn.on, .me-btn.on:hover { color: var(--danger-text); }
  /* server muted / deafened (#140): filled, not just red */
  .me-btn.server, .me-btn.server:hover { background: var(--danger); color: var(--on-danger); }
  .content { position: relative; flex: 1; min-width: 0; background: var(--bg-2); display: flex; flex-direction: column; }
  .empty { margin: auto; color: var(--text-3); }
  .join { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 12px; }
  .error { color: var(--danger-text); }
  .tab .pill { margin-left: 4px; min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
  .tab .invc { color: var(--accent-text); display: grid; margin-left: -2px; }
  .tab .dot { margin-left: 4px; width: 7px; height: 7px; border-radius: 50%; background: var(--text); display: inline-block; }
  /* the shared .toast look, under the channel header (it covered the search box there), centred by margins, not a
     transform (WebKitGTK blurs transformed text) */
  .notice { position: fixed; top: 112px; left: 0; right: 0; z-index: 40; width: max-content; max-width: min(520px, 80vw); margin: 0 auto; }
</style>
