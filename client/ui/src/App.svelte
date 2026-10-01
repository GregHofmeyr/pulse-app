<script lang="ts">
  import { onMount } from 'svelte'
  import { api, errorText } from './lib/tauri'
  import type { User } from './lib/protocol/User'
  import Login from './routes/Login.svelte'
  import Shell from './routes/Shell.svelte'

  let user = $state<User | null>(null)
  let booting = $state(true)
  let bootError = $state('')

  onMount(async () => {
    try {
      user = await api.restoreSession()
    } catch (e) {
      bootError = errorText(e)
    } finally {
      booting = false
    }
  })
</script>

{#if booting}
  <div class="boot" aria-busy="true"></div>
{:else if user}
  <Shell {user} onLogout={() => (user = null)} />
{:else}
  <Login notice={bootError} onAuthed={(u) => (user = u)} />
{/if}

<style>
  .boot { height: 100%; background: var(--bg-0); }
</style>
