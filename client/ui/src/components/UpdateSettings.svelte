<script lang="ts">
  // Settings › About › Updates (#460, moved from Advanced in #506): this Pulse's version and a "Check for updates" that asks the server now,
  // so a failed check is never silent. Only the Windows app updates itself; elsewhere updates come from the
  // package manager, and a build that doesn't update (a dev build) shows just its version.
  import { onMount } from 'svelte'
  import { api, errorText, type UpdateInfo } from '../lib/tauri'
  import { installUpdate, restartBlocked, updates } from '../lib/updates.svelte'

  let info = $state<UpdateInfo | null>(null)
  let checking = $state(false)
  /** What the last check here said: nothing yet, up to date, or why it couldn't check. */
  let result = $state<'none' | 'current' | 'failed'>('none')
  let error = $state('')

  onMount(async () => {
    try {
      info = await api.updateInfo()
    } catch {
      info = null
    }
  })

  async function check() {
    if (checking) return
    checking = true
    result = 'none'
    error = ''
    try {
      const offer = await api.updateCheck()
      // the title bar's "Restart to update" follows too
      updates.offer = offer
      result = offer ? 'none' : 'current'
    } catch (e) {
      error = errorText(e)
      result = 'failed'
    } finally {
      checking = false
    }
  }

  const blocked = $derived(restartBlocked())
  function restart() {
    if (blocked || updates.installing) return
    installUpdate(true).catch(() => {})
  }
</script>

{#if info}
  <div>
    <h3 class="settings-section-title">Updates</h3>
    <p class="version">Pulse {info.version}</p>
    {#if info.enabled}
      {#if updates.offer}
        <p class="ok" role="status">Pulse {updates.offer.version} is ready.</p>
        <button class="btn btn-primary" class:off={blocked} aria-disabled={blocked} disabled={updates.installing}
          data-tip={blocked ? 'Finish your call first' : undefined} onclick={restart}>Restart to update</button>
        {#if blocked}<p class="hint">Finish your call first.</p>{/if}
        {#if updates.error}<p class="err" role="alert">Couldn't update: {updates.error}</p>{/if}
      {:else}
        <button class="btn btn-secondary" disabled={checking} onclick={check}>{checking ? 'Checking…' : 'Check for updates'}</button>
        {#if result === 'current'}<p class="ok" role="status">You're up to date.</p>{/if}
        {#if result === 'failed'}<p class="err" role="alert">Couldn't check for updates: {error}</p>{/if}
      {/if}
    {:else if info.packageManager}
      <p class="hint">Updates come from your package manager.</p>
    {/if}
  </div>
{/if}

<style>
  .version { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  .ok { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  button + .ok, button + .err, button + .hint { margin: 10px 0 0; }
  .hint { margin: 0; font-size: 12px; color: var(--text-3); }
  .err { margin: 10px 0 0; font-size: 13px; color: var(--danger-text); }
  .off { opacity: .5; cursor: not-allowed; }
</style>
