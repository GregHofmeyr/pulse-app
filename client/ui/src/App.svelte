<script lang="ts">
  import { onMount } from 'svelte'
  import { startInviteLinks } from './lib/inviteLinks.svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { api, errorText } from './lib/tauri'
  import { app, resetState, startListening } from './lib/store.svelte'
  import { startVoiceListening } from './lib/voice.svelte'
  import { badgeStore, startBadgeListening } from './lib/badges.svelte'
  import { startSoundboardListening } from './lib/soundboard.svelte'
  // the ring store follows events and Ready from the start, whatever is on screen (#50)
  import './lib/calls.svelte'
  import { installTooltips } from './lib/tooltip'
  import { watchWindowFocus, windowFocus } from './lib/windowFocus.svelte'
  import type { User } from './lib/protocol/User'
  import Login from './routes/Login.svelte'
  import Shell from './routes/Shell.svelte'
  import ShellBoundary from './components/ShellBoundary.svelte'
  import UpdateRequired from './routes/UpdateRequired.svelte'
  import Waiting from './routes/Waiting.svelte'
  import Updating from './routes/Updating.svelte'
  import DownloadAnnouncer from './components/DownloadAnnouncer.svelte'
  import { screenFor } from './lib/screen'
  import { installUpdate, restartBlocked, startIdleInstall, startUpdateListening, updates } from './lib/updates.svelte'
  import UpdateFailed from './components/UpdateFailed.svelte'
  import { hasUnsentDrafts } from './components/Composer.svelte'
  import { stream } from './lib/stream/stream.svelte'
  import type { FailedUpdate } from './lib/tauri'
  import { startMediaActivity } from './lib/activityPrefs.svelte'
  import { startGameActivity } from './lib/gamePrefs.svelte'
  import { tellClipDrop } from './lib/clipDropPrefs.svelte'
  import { startSteamListening } from './lib/steam.svelte'

  let user = $state<User | null>(null)
  let booting = $state(true)
  let bootError = $state('')
  // restore found a stored session but the server turned the connection away: wait for Ready
  let restoreRefused = $state(false)
  const screen = $derived(screenFor({ booting, signedIn: !!user, conn: app.state.conn, restoreRefused }))

  onMount(() => installTooltips())
  // #460: an update found while running installs by itself once Pulse is left alone, never over anything going on
  onMount(() =>
    startIdleInstall(
      () =>
        restartBlocked() ||
        stream.live !== null ||
        hasUnsentDrafts() ||
        Object.values(app.state.pending).some((list) => list.length > 0) ||
        Object.values(app.state.clipJobs).some((j) => j.stage === 'preparing' || j.stage === 'uploading' || j.stage === 'posting'),
    ),
  )
  // #460: the last install didn't finish: say so, with a download link (once)
  let failedUpdate = $state<FailedUpdate | null>(null)
  onMount(watchWindowFocus)

  // While the window is unfocused (app.css `.unfocused`) nothing animates in the background, on any screen.
  $effect(() => {
    if (windowFocus.focused) return
    document.body.classList.add('unfocused')
    return () => document.body.classList.remove('unfocused')
  })

  // Startup timeline: tell the core when the first real screen has painted (once).
  let painted = false
  $effect(() => {
    if (painted || screen === 'boot') return
    painted = true
    const shown = screen
    requestAnimationFrame(() => void invoke('ui_painted', { screen: shown }).catch(() => {}))
  })

  onMount(() => {
    let off: (() => void) | undefined
    // Listen before restoring so the first Ready can't slip past us.
    // "Listening to" (#275): the core reads nothing until it's told the saved switches
    const media = startMediaActivity().catch(() => () => {})
    // "Playing" (#380): likewise, nothing is looked for until the core has the saved switches
    const games = startGameActivity().catch(() => () => {})
    // Clip drop (#382): its own switch, told apart from Game activity; nothing is watched until then
    tellClipDrop()
    Promise.all([startListening(), startVoiceListening(), startUpdateListening(), media, games]).then(async ([unlisten, unlistenVoice, unlistenUpdates, unlistenMedia, unlistenGames]) => {
      const unlistenBadges = startBadgeListening()
      const unlistenSounds = startSoundboardListening()
      const unlistenSteam = startSteamListening()
      const invites = startInviteLinks().catch(() => () => {})
      off = () => {
        void invites.then((f) => f())
        unlisten()
        unlistenVoice()
        unlistenUpdates()
        unlistenBadges()
        unlistenSounds()
        unlistenMedia()
        unlistenGames()
        unlistenSteam()
      }
      // A newer Pulse (#82): the check started with the app; install it now ("Updating Pulse…", then Pulse reopens
      // on it). No offer, or a failed install, boots as usual on this version.
      const offer = await api.updateLaunch().catch(() => null)
      if (offer) {
        updates.offer = offer
        try {
          await installUpdate(false)
          return // Pulse is closing for the installer
        } catch {
          // logged by installUpdate; carry on as this version
        }
      }
      api.updateFailed().then((f) => (failedUpdate = f ?? null), () => {})
      try {
        const r = await api.restoreSession()
        user = r.status === 'ready' ? r.user : null
        restoreRefused = r.status === 'refused'
      } catch (e) {
        bootError = errorText(e)
      } finally {
        booting = false
      }
    })
    return () => off?.()
  })

  // The refused startup got in: Ready carries who we are.
  $effect(() => {
    if (restoreRefused && !user && app.state.me && app.state.conn === 'connected') {
      user = app.state.me
      restoreRefused = false
    }
  })

  // The user gave up waiting: sign out (the core forgets the session even if the server can't be
  // told) and go to login.
  async function signOutWhileWaiting() {
    try {
      await api.logout()
    } finally {
      user = null
      restoreRefused = false
      resetState()
      badgeStore.reset()
      bootError = ''
    }
  }

  // The server ended our session (logout elsewhere, expiry): back to login.
  $effect(() => {
    if ((user || restoreRefused) && app.state.conn === 'logged_out') {
      user = null
      restoreRefused = false
      resetState()
      badgeStore.reset()
      bootError = 'You were signed out. Please log in again.'
    }
  })
</script>

{#if screen === 'boot'}
  <!-- the app's own background and mark while the core restores the session (the window shows before the page) -->
  <div class="boot" aria-busy="true" aria-label="Starting Pulse" data-tauri-drag-region>
    <span class="boot-mark" aria-hidden="true">
      <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
    </span>
    <span class="boot-name">Pulse</span>
  </div>
{:else if screen === 'update'}
  <UpdateRequired />
{:else if screen === 'waiting'}
  <Waiting onSignOut={signOutWhileWaiting} />
{:else if screen === 'shell' && user}
  <!-- #132: an error showing the Shell gets "Something went wrong" and a Reload, not a half-dead window -->
  <ShellBoundary>
    <Shell {user} onLogout={() => { user = null; restoreRefused = false; resetState(); badgeStore.reset() }} />
  </ShellBoundary>
{:else}
  <Login notice={bootError} onAuthed={(u) => { bootError = ''; restoreRefused = false; user = u }} />
{/if}
{#if failedUpdate && !updates.installing}<UpdateFailed failed={failedUpdate} onClose={() => (failedUpdate = null)} />{/if}
<!-- #82: over everything, so a failed install leaves the screen beneath exactly as it was -->
{#if updates.installing}<Updating />{/if}
<!-- #205: downloads starting and finishing, for screen readers -->
<DownloadAnnouncer />

<style>
  .boot { height: 100%; background: var(--bg-0); display: grid; place-content: center; justify-items: center; gap: 14px; }
  /* fades in only if startup takes a moment; then breathes gently (opacity only), still under reduced motion */
  .boot-mark, .boot-name { animation: boot-in var(--dur-slow) var(--ease) var(--dur-fast) both; }
  .boot-mark { width: 56px; height: 56px; border-radius: var(--radius-xl); background: var(--accent); color: var(--on-accent); display: grid; place-items: center; }
  .boot-mark svg { animation: boot-breathe calc(var(--dur-slow) * 4) var(--ease) infinite alternate; }
  .boot-name { font-size: 15px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  @keyframes boot-in { from { opacity: 0; transform: scale(.96); } to { opacity: 1; transform: none; } }
  @keyframes boot-breathe { from { opacity: 1; } to { opacity: .55; } }
  @media (prefers-reduced-motion: reduce) {
    .boot-mark, .boot-name, .boot-mark svg { animation: none; }
  }
</style>
