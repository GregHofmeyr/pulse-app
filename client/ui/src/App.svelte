<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { api, errorText } from './lib/tauri'
  import { app, resetState, startListening } from './lib/store.svelte'
  import { startVoiceListening } from './lib/voice.svelte'
  import { installTooltips } from './lib/tooltip'
  import { watchWindowFocus, windowFocus } from './lib/windowFocus.svelte'
  import type { User } from './lib/protocol/User'
  import Login from './routes/Login.svelte'
  import Shell from './routes/Shell.svelte'
  import UpdateRequired from './routes/UpdateRequired.svelte'
  import { screenFor } from './lib/screen'

  let user = $state<User | null>(null)
  let booting = $state(true)
  let bootError = $state('')
  const screen = $derived(screenFor({ booting, signedIn: !!user, conn: app.state.conn }))

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
    Promise.all([startListening(), startVoiceListening()]).then(async ([unlisten, unlistenVoice]) => {
      off = () => {
        unlisten()
        unlistenVoice()
      }
      try {
        user = await api.restoreSession()
      } catch (e) {
        bootError = errorText(e)
      } finally {
        booting = false
      }
    })
    return () => off?.()
  })

  // The server ended our session (logout elsewhere, expiry): back to login.
  $effect(() => {
    if (user && app.state.conn === 'logged_out') {
      user = null
      resetState()
      bootError = 'You were signed out. Please log in again.'
    }
  })
</script>

{#if screen === 'boot'}
  <div class="boot" aria-busy="true"></div>
{:else if screen === 'update'}
  <UpdateRequired />
{:else if screen === 'shell' && user}
  <Shell {user} onLogout={() => { user = null; resetState() }} />
{:else}
  <Login notice={bootError} onAuthed={(u) => { bootError = ''; user = u }} />
{/if}

<style>
  .boot { height: 100%; background: var(--bg-0); }
</style>
