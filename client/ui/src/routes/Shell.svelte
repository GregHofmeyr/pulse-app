<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { listen } from '@tauri-apps/api/event'
  import ConnText from '../components/ConnText.svelte'
  import Icon from '../components/Icon.svelte'
  import ChannelList from '../components/ChannelList.svelte'
  import MemberList from '../components/MemberList.svelte'
  import MiniPreview from '../components/MiniPreview.svelte'
  import StreamToast from '../components/StreamToast.svelte'
  import VoiceStatus from '../components/VoiceStatus.svelte'
  import VoicePanel from '../components/VoicePanel.svelte'
  import TextChannel from '../components/TextChannel.svelte'
  import ClipsChannel from '../components/ClipsChannel.svelte'
  import Settings, { type Section } from '../components/Settings.svelte'
  import FullProfile from '../components/FullProfile.svelte'
  import NicknameDialog from '../components/NicknameDialog.svelte'
  import ServerSettings from '../components/ServerSettings.svelte'
  import { clickOutside, narrowerThan } from '../lib/actions'
  import { NARROW_BODY, searchMakesRoom } from '../lib/layout'
  import ReportBug from '../components/ReportBug.svelte'
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
  import StatusPicker from '../components/StatusPicker.svelte'
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
  import { MUTE_FOR, muteUntil } from '../lib/muteDurations'

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
  let menuOpen = $state(false)
  /** Your status menu (#45), opened from your avatar: the avatar it came from. */
  let statusMenu = $state<HTMLElement | null>(null)
  let menuToggle = $state<HTMLButtonElement>()
  /** An outside press or Esc closes the account menu; after Esc, focus goes back to its toggle. */
  function closeAccountMenu(e: Event) {
    menuOpen = false
    if (e instanceof KeyboardEvent) menuToggle?.focus()
  }
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
  const searchAside = $derived(searchMakesRoom({ narrow: narrowBody, searchOpen: search.open, threadOpen: !!ui.openThreadId }))
  const hideRight = $derived((fullShown && !panels.right) || searchAside)
  let sidePanel = $state<HTMLElement>()
  let rightPanel = $state<HTMLElement>()
  // Before a panel hides: a menu open in it closes (an unseen menu would take Esc first), and focus in
  // it moves to the toggle that brings it back (or it would fall to the top of the page).
  $effect.pre(() => {
    const [left, right] = [hideLeft, hideRight]
    untrack(() => {
      if (left) {
        menuOpen = false
        serverMenuOpen = false
      }
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
    toggle: (side) => (panels[side] = !panels[side]),
  })
  let serverMenuOpen = $state(false)
  let serverToggle = $state<HTMLButtonElement>()
  /** The same for the server menu. */
  function closeServerMenu(e: Event) {
    serverMenuOpen = false
    if (e instanceof KeyboardEvent) serverToggle?.focus()
  }
  let serverSettingsOpen = $state(false)
  // Both belong to the server they were opened on.
  $effect(() => {
    void activeServerId
    serverMenuOpen = false
    serverSettingsOpen = false
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
  $effect(() => {
    ui.openChannelId = activeChannelId
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

  // Right-click a server tab: mute/unmute the whole server.
  let serverMenu = $state<null | { id: string; x: number; y: number }>(null)
  const serverMuted = (id: string) =>
    app.state.mutes.some((m) => m.target_kind === 'server' && m.target_id === id && (m.until === null || m.until > nowIso))
  async function muteServer(id: string, minutes: number | null | 'off') {
    serverMenu = null
    try {
      if (minutes === 'off') await api.clearMute('server', id)
      else await api.setMute('server', id, muteUntil(minutes))
    } catch (e) {
      error = errorText(e)
    }
  }

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
    // the top text channel, as the sidebar lists them
    const general = Object.values(app.state.channels)
      .filter((c) => c.server_id === id && c.kind === 'text')
      .sort((a, b) => a.position - b.position || (a.id < b.id ? -1 : 1))[0]
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

<div class="app">
  <header data-tauri-drag-region>
    <!-- the logo egg (#49): 7 quick clicks; hidden on purpose, so no button or keyboard path -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="logo" aria-hidden="true" onclick={() => logoClick()}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
    </span>
    <nav aria-label="Servers">
      <button class="tab" class:active={activeServerId === null} onclick={() => { activeServerId = null; activeChannelId = null }}>
        <Icon name="home" size={16} /> Home{#if home > 0}<span class="pill">{home > 99 ? '99+' : home}</span>{/if}
      </button>
      <span class="sep"></span>
      {#each servers as s (s.id)}
        {@const badge = serverBadge(app.state, s.id, nowIso)}
        {@const talking = inVoice(app.state, s.id)}
        <button class="tab" class:active={activeServerId === s.id} onclick={() => selectServer(s.id)}
          oncontextmenu={(e) => { e.preventDefault(); serverMenu = { id: s.id, x: e.clientX, y: e.clientY } }}>
          <span class="chip" style:background={avatarColor(s.id)}>{initial(s.name)}</span>{s.name}
          {#if talking > 0}<span class="invc" aria-label="{talking} in voice" data-tip="{talking} in voice"><Icon name="speaker" size={13} /></span>{/if}
          {#if badge.mentions > 0}<span class="pill">{badge.mentions}</span>{:else if badge.dot}<span class="dot" aria-label="new messages"></span>{/if}
        </button>
      {/each}
      <button class="add" aria-label="Create a server" title="Create a server" onclick={() => (creating = true)}><Icon name="plus" size={16} /></button>
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
    <button class="win" aria-label="Minimise" onclick={() => win.minimize()}><Icon name="min" size={14} /></button>
    <button class="win" aria-label="Maximise" onclick={() => win.toggleMaximize()}><Icon name="square" size={13} /></button>
    <button class="win" aria-label="Close" onclick={() => win.close()}><Icon name="x" size={14} /></button>
  </header>

  <div class="body" use:narrowerThan={{ px: NARROW_BODY, set: (v) => (narrowBody = v) }}>
    <aside id={PANEL_ID.left} class="panel side" hidden={hideLeft} bind:this={sidePanel}>
      <div class="panel-head">
        {#if activeServer}
          <button bind:this={serverToggle} class="head-btn" data-menu-toggle aria-haspopup="menu" aria-expanded={serverMenuOpen}
            onclick={() => (serverMenuOpen = !serverMenuOpen)}>{activeServer.name}<Icon name="chevron" size={14} /></button>
          {#if serverMenuOpen}
            <div class="menu head-menu menu-surface pop-in" role="menu" use:clickOutside={closeServerMenu}>
              <button class="menu-item" role="menuitem" onclick={() => { serverMenuOpen = false; serverSettingsOpen = true }}>Server settings</button>
            </div>
          {/if}
        {:else}Direct messages{/if}
      </div>
      {#if activeServerId}
        <ChannelList serverId={activeServerId} {activeChannelId} voiceChannelId={voice.channelId} speaking={ringing}
          onSelect={(id) => (activeChannelId = id)}
          onJoinVoice={(id) => { activeChannelId = id; if (voice.channelId !== id && member) void voiceApi.join(id) }}
          onMessage={messageUser} onVoiceSettings={() => openSettings('voice')} />
      {:else}
        <ConversationList activeId={activeChannelId} onOpen={openConversation} onNew={() => (picker = { mode: 'new' })}
          onLeft={(id) => { if (activeChannelId === id) activeChannelId = null }} />
      {/if}
      <VoiceStatus />
      <div class="me">
        <!-- your avatar opens your status menu (#45), whose Your profile opens your quick card; the rest opens the account menu -->
        <button type="button" class="me-avatar" aria-label={badgeStore.unseen > 0 ? 'Your status and profile (new badges)' : 'Your status and profile'} data-tip="Status and profile"
          data-menu-toggle aria-haspopup="menu" aria-expanded={!!statusMenu} onclick={(e) => (statusMenu = statusMenu ? null : e.currentTarget)}><Avatar id={me.id} name={baseName(me)} size={34} presence={presenceOf(app.state, me.id, clock.nowIso)} />{#if badgeStore.unseen > 0}<span class="new-dot" aria-hidden="true"></span>{/if}</button>
        {#if statusMenu}
          {@const opener = statusMenu}
          <StatusPicker {opener} onClose={() => (statusMenu = null)} onProfile={() => openCard(me.id, null, opener)} />
        {/if}
        <button bind:this={menuToggle} class="who-btn" data-tip="Account" data-menu-toggle aria-haspopup="menu" aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>
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
        {#if menuOpen}
          <div class="menu menu-surface pop-in" role="menu" use:clickOutside={closeAccountMenu}>
            <button class="menu-item" role="menuitem" onclick={() => { menuOpen = false; openSettings() }}>Settings</button>
            <button class="menu-item warn" role="menuitem" onclick={() => { menuOpen = false; reportOpen = true }}>Report a bug</button>
            <button class="menu-item danger" role="menuitem" onclick={logout}>Log out</button>
          </div>
        {/if}
      </div>
    </aside>

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
          onLeft={() => (activeChannelId = null)} onStartQoty={() => (qotyStart = activeServerId)} />{/key}
      {:else if activeServerId === null}
        <PeopleBoard onOpen={openConversation} onJoinVoice={(ch, srv) => { selectServer(srv); activeChannelId = ch; if (voice.channelId !== ch) void voiceApi.join(ch) }} />
      {:else}
        <p class="empty">Pick a channel.</p>
      {/if}
      {#each mini ? [mini] : [] as m (m.key)}<MiniPreview channelId={m.channelId} streamer={m.streamer} />{/each}
    </main>

    <!-- one line: nothing but the list inside, so an empty wrapper (Home) matches :empty and takes no gap -->
    <div id={PANEL_ID.right} class="right-panel" hidden={hideRight} bind:this={rightPanel}>{#if search.open}<SearchResults />{:else if activeServerId}<MemberList serverId={activeServerId} onMessage={messageUser} />{:else if activeChannel?.kind === 'group' && activeChannelId}<GroupMembers channelId={activeChannelId} />{/if}</div>
  </div>
</div>
<StreamToast />
<BadgeToast />
{#if serverSettingsOpen && activeServer}<ServerSettings server={activeServer} onClose={() => (serverSettingsOpen = false)} />{/if}
{#if settingsOpen}<Settings section={settingsSection} onClose={() => { settingsOpen = false; void checkKeybinds() }} />{/if}
{#if goLiveUi.picked && goLiveUi.channelId}<GoLiveDialog channelId={goLiveUi.channelId} bind:picked={goLiveUi.picked} onClose={closeGoLive} />{/if}
{#if qotyStart}<QotyStart serverId={qotyStart} onClose={() => (qotyStart = null)} />{/if}
{#if reportOpen}<ReportBug onClose={() => (reportOpen = false)} />{/if}
<!-- Profiles: the quick card (one at a time), and the full profile's place (Task 12 renders it here) -->
<ProfileCardPopover />
{#if profileUi.full}<FullProfile userId={profileUi.full.userId} serverId={profileUi.full.serverId} onClose={closeFull} />{/if}
{#if profileUi.rename}<NicknameDialog target={profileUi.rename} onClose={closeRename} />{/if}
{#if creating}
  <div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" onclick={() => (creating = false)}></div>
  <form class="modal dialog pop-in" onsubmit={createServer} aria-label="Create a server">
    <h2 class="modal-title">Create a server</h2>
    <label>NAME <input class="input" bind:value={newName} maxlength="64" required placeholder="Game Night" /></label>
    {#if createError}<p class="error">{createError}</p>{/if}
    <div class="dialog-actions">
      <button type="button" class="btn btn-lg btn-secondary" onclick={() => (creating = false)}>Cancel</button>
      <button class="btn btn-lg btn-primary" disabled={!newName.trim()}>Create</button>
    </div>
  </form>
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
{#if notice}<div class="notice fade-in" role="status">{notice}</div>{/if}
{#if serverMenu}
  {@const id = serverMenu.id}
  <div class="ctx-backdrop" role="presentation" onclick={() => (serverMenu = null)}></div>
  <div class="ctx menu-surface pop-in" role="menu" use:clickOutside={() => (serverMenu = null)} style:left="{serverMenu.x}px" style:top="{serverMenu.y}px">
    {#if serverMuted(id)}
      <button class="menu-item" role="menuitem" onclick={() => muteServer(id, 'off')}>Unmute server</button>
    {:else}
      <span class="menu-label">Mute server</span>
      {#each MUTE_FOR as d (d.label)}
        <button class="menu-item" role="menuitem" onclick={() => muteServer(id, d.minutes)}>{d.label}</button>
      {/each}
    {/if}
  </div>
{/if}

<style>
  .app { height: 100%; display: flex; flex-direction: column; background: var(--bg-0); }
  header { height: 52px; flex-shrink: 0; display: flex; align-items: center; gap: 6px; padding: 0 10px 0 12px; }
  .logo { width: 30px; height: 30px; border-radius: 8px; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; margin-right: 8px; }
  nav { display: flex; align-items: center; gap: 6px; min-width: 0; overflow: hidden; }
  .tab { height: 36px; padding: 0 12px 0 10px; border-radius: 10px; border: 1px solid transparent; background: transparent; color: var(--text-2); display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; white-space: nowrap; }
  .tab:hover { background: var(--bg-2); }
  /* background and colour only: a bolder weight would widen the tab and nudge the ones after it */
  .tab.active { background: var(--bg-3); border-color: var(--bg-4); color: var(--text); }
  .chip { width: 22px; height: 22px; border-radius: 6px; color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 4px; }
  .grow { flex: 1; align-self: stretch; }
  .conn { font-size: 12px; font-weight: 600; color: var(--warn-text); padding: 4px 10px; border-radius: 12px; background: var(--warn-soft); margin-right: 6px; }
  .win { width: 32px; height: 28px; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .win:hover { background: var(--bg-2); color: var(--text); }
  .body { flex: 1; min-height: 0; display: flex; gap: 8px; padding: 0 8px 8px; }
  .panel { border-radius: var(--radius); overflow: hidden; }
  .side { width: 264px; flex-shrink: 0; background: var(--bg-1); display: flex; flex-direction: column; }
  /* hidden, not removed: what the panels hold (an open menu, a draft) survives full view */
  .side[hidden] { display: none; }
  /* a real box (aria-controls points at it; a display:contents one leaves the accessibility tree) */
  .right-panel { display: flex; flex-shrink: 0; min-height: 0; }
  .right-panel[hidden], .right-panel:empty { display: none; }
  .panel-head { position: relative; height: 52px; flex-shrink: 0; padding: 0 16px; display: flex; align-items: center; font-size: 15px; font-weight: 600; border-bottom: 1px solid var(--line-soft); }
  /* the hover is a rounded pill inside the header, with the text where the padding put it */
  .head-btn { flex: 1; min-width: 0; height: 40px; margin: 0 -8px; padding: 0 8px; border-radius: 10px; display: flex; align-items: center; justify-content: space-between; gap: 8px; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; }
  .head-btn:hover { background: var(--bg-2); }
  /* two classes: .menu (declared below, for the upward account menu) must not win `bottom` back */
  .menu.head-menu { left: 8px; right: 8px; top: calc(100% + 4px); bottom: auto; }
  .me { padding: 10px 10px 10px 12px; background: var(--bg-inset); display: flex; align-items: center; gap: 10px; }
  .who { flex: 1; display: flex; flex-direction: column; font-size: 13px; font-weight: 600; }
  .who small { font-size: 12px; font-weight: 400; color: var(--text-3); }
  .add { width: 36px; height: 36px; flex-shrink: 0; border: 1px dashed var(--bg-4); border-radius: 10px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .add:hover { color: var(--text); border-color: var(--text-3); }
  .me { position: relative; }
  .me-avatar { position: relative; flex-shrink: 0; display: flex; padding: 0; border: 0; border-radius: 50%; background: none; }
  /* unseen badges (#49): a small dot until you open your trophy case */
  .new-dot { position: absolute; top: -1px; right: -1px; width: 10px; height: 10px; border-radius: 50%; background: var(--badge-new); box-shadow: 0 0 0 2px var(--ring); }
  .who-btn { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 4px 6px; border: 0; border-radius: 10px; background: transparent; text-align: left; }
  .who-btn:hover { background: var(--bg-2); }
  .who-btn .who { overflow: hidden; }
  .menu { position: absolute; left: 8px; bottom: calc(100% + 6px); z-index: 10; min-width: 200px; }
  /* not the bold of the header it can sit under */
  .menu button { font-weight: 400; }
  .menu .warn { color: var(--warn-text); }
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
  .tab .pill { margin-left: 4px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-grid; place-items: center; }
  .tab .invc { color: var(--accent-text); display: grid; margin-left: -2px; }
  .tab .dot { margin-left: 4px; width: 7px; height: 7px; border-radius: 50%; background: var(--text); display: inline-block; }
  .notice { position: fixed; top: 56px; left: 50%; transform: translateX(-50%); z-index: 40; padding: 8px 14px; border-radius: 10px; background: var(--bg-3); color: var(--text); font-size: 13px; box-shadow: var(--shadow-2); }
  .ctx-backdrop { position: fixed; inset: 0; z-index: 44; }
  .ctx { position: fixed; z-index: 45; width: 210px; }
</style>
