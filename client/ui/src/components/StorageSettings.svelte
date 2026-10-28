<script lang="ts">
  // Server settings → Storage: this server's clips, their size and what they'd cost at R2's list price.
  import { api, errorText } from '../lib/tauri'
  import { formatBytes } from '../lib/clips'
  import type { Server } from '../lib/protocol/Server'
  import type { ClipStorage } from '../lib/protocol/ClipStorage'

  let { server }: { server: Server } = $props()

  let usage = $state<ClipStorage | null>(null)
  let error = $state('')
  $effect(() => {
    const id = server.id
    usage = null
    error = ''
    let stale = false
    api.clipStorage(id).then(
      (u) => { if (!stale) usage = u },
      (e) => { if (!stale) error = errorText(e) },
    )
    return () => { stale = true }
  })

  const cost = (usd: number) => (usd < 0.01 ? 'under $0.01' : `about $${usd.toFixed(2)}`)
</script>

<h2>Storage</h2>
<h3>Clips</h3>
{#if error}
  <p class="error" role="alert">{error}</p>
{:else if !usage}
  <p class="loading">Loading…</p>
{:else}
  <p>{usage.count} {usage.count === 1 ? 'clip' : 'clips'} · {formatBytes(usage.bytes)} · {cost(usage.est_monthly_usd)} a month at R2's list price</p>
{/if}

<style>
  h2 { margin: 0 0 8px; font-size: 20px; }
  h3 { margin: 16px 0 6px; font-size: 14px; }
  p { margin: 0; color: var(--text-2); }
  .error { margin: 0 0 12px; color: var(--danger); font-size: 13px; }
</style>
