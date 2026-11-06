<script lang="ts" module>
  /** Not published yet: how often to look again (#82). */
  export const FORCED_RECHECK_MS = 60_000
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { api, errorText } from '../lib/tauri'
  import { installUpdate, updates } from '../lib/updates.svelte'

  // The server needs a newer Pulse (#82): install it by itself ("Updating Pulse…" shows over this screen; Pulse
  // reopens on it). Not published yet: say so and look again every minute. A failure: say why, with Retry.
  let phase = $state<'checking' | 'installing' | 'not-yet' | 'failed'>('checking')
  let error = $state('')
  let timer: ReturnType<typeof setTimeout> | undefined
  let gone = false

  async function check() {
    clearTimeout(timer)
    phase = 'checking'
    let offer
    try {
      offer = await api.updateCheck()
    } catch (e) {
      if (gone) return
      error = errorText(e)
      phase = 'failed'
      return
    }
    if (gone) return
    if (!offer) {
      phase = 'not-yet'
      timer = setTimeout(check, FORCED_RECHECK_MS)
      return
    }
    updates.offer = offer
    phase = 'installing'
    try {
      await installUpdate(false)
    } catch {
      if (gone) return
      error = updates.error ?? 'Something went wrong'
      phase = 'failed'
    }
  }

  onMount(() => void check())
  onDestroy(() => {
    gone = true
    clearTimeout(timer)
  })
</script>

<main data-tauri-drag-region>
  <div class="card" role="alert">
    <h1>plz update :)</h1>
    {#if phase === 'not-yet'}
      <p>Pulse needs an update that isn't ready yet. It'll install as soon as it is.</p>
    {:else if phase === 'failed'}
      <p>Couldn't update: {error}</p>
    {:else}
      <p>Pulse has a new version. Getting it now…</p>
    {/if}
    {#if phase === 'not-yet' || phase === 'failed'}
      <button class="btn btn-secondary" onclick={() => void check()}>Retry</button>
    {/if}
  </div>
</main>

<style>
  main { height: 100%; display: grid; place-items: center; background: var(--bg-0); }
  .card { width: 340px; padding: 28px; border-radius: var(--radius); background: var(--bg-1); text-align: center; display: flex; flex-direction: column; align-items: center; gap: 16px; }
  h1 { margin: 0 0 -6px; font-size: 22px; }
  p { margin: 0; color: var(--text-2); font-size: 14px; line-height: 1.5; overflow-wrap: anywhere; }
</style>
