<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { api, errorText } from './lib/tauri'
  import { app, resetState, startListening } from './lib/store.svelte'
  import { startVoiceListening } from './lib/voice.svelte'
  import { installTooltips } from './lib/tooltip'
  import type { User } from './lib/protocol/User'
  import Login from './routes/Login.svelte'
  import Shell from './routes/Shell.svelte'
  import UpdateRequired from './routes/UpdateRequired.svelte'
  import Waiting from './routes/Waiting.svelte'
  import { screenFor } from './lib/screen'

  let user = $state<User | null>(null)
  let booting = $state(true)
  let bootError = $state('')
  // restore found a stored session but the server turned the connection away: wait for Ready
  let restoreRefused = $state(false)
  const screen = $derived(screenFor({ booting, signedIn: !!user, conn: app.state.conn, restoreRefused }))

  onMount(() => installTooltips())

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
    Promise.all([startListening(), startVoiceListening()]).then(async ([unlisten, unlistenVoice]) => {
      off = () => {
        unlisten()
        unlistenVoice()
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

  // The server ended our session (logout elsewhere, expiry): back to login.
  $effect(() => {
    if ((user || restoreRefused) && app.state.conn === 'logged_out') {
      user = null
      restoreRefused = false
      resetState()
      bootError = 'You were signed out. Please log in again.'
    }
  })
</script>

{#if screen === 'boot'}
  <div class="boot" aria-busy="true"></div>
{:else if screen === 'update'}
  <UpdateRequired />
{:else if screen === 'waiting'}
  <Waiting />
{:else if screen === 'shell' && user}
  <Shell {user} onLogout={() => { user = null; restoreRefused = false; resetState() }} />
{:else}
  <Login notice={bootError} onAuthed={(u) => { bootError = ''; restoreRefused = false; user = u }} />
{/if}

<style>
  .boot { height: 100%; background: var(--bg-0); }
</style>
