<script lang="ts">
  import { onMount } from 'svelte'
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
  import UpdateRequired from './routes/UpdateRequired.svelte'
  import Waiting from './routes/Waiting.svelte'
  import Updating from './routes/Updating.svelte'
  import DownloadAnnouncer from './components/DownloadAnnouncer.svelte'
  import { screenFor } from './lib/screen'
  import { installUpdate, startUpdateListening, updates } from './lib/updates.svelte'
  import { startMediaActivity } from './lib/activityPrefs.svelte'

  let user = $state<User | null>(null)
  let booting = $state(true)
  let bootError = $state('')
  // restore found a stored session but the server turned the connection away: wait for Ready
  let restoreRefused = $state(false)
  const screen = $derived(screenFor({ booting, signedIn: !!user, conn: app.state.conn, restoreRefused }))

  onMount(() => installTooltips())
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
    Promise.all([startListening(), startVoiceListening(), startUpdateListening(), media]).then(async ([unlisten, unlistenVoice, unlistenUpdates, unlistenMedia]) => {
      const unlistenBadges = startBadgeListening()
      const unlistenSounds = startSoundboardListening()
      off = () => {
        unlisten()
        unlistenVoice()
        unlistenUpdates()
        unlistenBadges()
        unlistenSounds()
        unlistenMedia()
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
  <div class="boot" aria-busy="true"></div>
{:else if screen === 'update'}
  <UpdateRequired />
{:else if screen === 'waiting'}
  <Waiting onSignOut={signOutWhileWaiting} />
{:else if screen === 'shell' && user}
  <Shell {user} onLogout={() => { user = null; restoreRefused = false; resetState(); badgeStore.reset() }} />
{:else}
  <Login notice={bootError} onAuthed={(u) => { bootError = ''; restoreRefused = false; user = u }} />
{/if}
<!-- #82: over everything, so a failed install leaves the screen beneath exactly as it was -->
{#if updates.installing}<Updating />{/if}
<!-- #205: downloads starting and finishing, for screen readers -->
<DownloadAnnouncer />

<style>
  .boot { height: 100%; background: var(--bg-0); }
</style>
