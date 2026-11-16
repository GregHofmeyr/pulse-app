<script lang="ts">
  // Auto clip drop's toasts (#382): "New clip: <file> — Share to #clips?" for each recording the core offers, with
  // Share (opens that clips channel's editor on it) and Dismiss. With several clips channels and none used before,
  // the channel is picked here. Sits in the stream toasts' stack (StreamToast).
  import { onMount } from 'svelte'
  import Select from './Select.svelte'
  import { exitFade } from '../lib/exit'
  import { clipDrop, clipsTarget, dismissOffer, shareOffer, startClipDrop } from '../lib/clipDrop.svelte'
  import { app } from '../lib/store.svelte'
  import type { ClipOffer } from '../lib/tauri'

  onMount(() => {
    let off: (() => void) | undefined
    let gone = false
    startClipDrop().then((f) => (gone ? f() : (off = f)), () => {})
    return () => {
      gone = true
      off?.()
    }
  })

  const target = $derived(clipsTarget(app.state, clipDrop.last))
  const serverName = (id: string | null) => app.state.servers.find((s) => s.id === id)?.name ?? ''
  const options = $derived(target.kind === 'ask' ? target.choices.map((c) => ({ value: c.id, label: `#${c.name ?? ''} · ${serverName(c.server_id)}` })) : [])
  /** The channel picked in an offer's toast, by its path (the first until one is picked). */
  let picked = $state<Record<string, string | null>>({})

  function share(o: ClipOffer) {
    const id = target.kind === 'channel' ? target.channel.id : target.kind === 'ask' ? (picked[o.path] ?? target.choices[0].id) : null
    if (id) shareOffer(o, id)
  }
</script>

{#each clipDrop.offers as o (o.path)}
  <div class="toast clip-offer fade-in" role="status" out:exitFade>
    {#if target.kind === 'none'}
      <span class="line">New clip: <strong>{o.name}</strong>. Join a server with a clips channel to share it.</span>
    {:else}
      <span class="line">New clip: <strong>{o.name}</strong> — Share to {target.kind === 'channel' ? `#${target.channel.name ?? ''}` : 'a clips channel'}?</span>
      {#if target.kind === 'ask'}
        <div class="pick"><Select bind:value={picked[o.path]} {options} label="Clips channel" drop="up" /></div>
      {/if}
      <button type="button" class="btn btn-primary" onclick={() => share(o)}>Share</button>
    {/if}
    <button type="button" class="btn btn-secondary" onclick={() => dismissOffer(o.path)}>Dismiss</button>
  </div>
{/each}

<style>
  /* the shared .toast (app.css); a long file name wraps rather than widening the stack past the window */
  .clip-offer { max-width: min(520px, calc(100vw - 36px)); flex-wrap: wrap; }
  .line { flex: 1 1 200px; min-width: 0; overflow-wrap: anywhere; }
  .pick { min-width: 180px; }
</style>
