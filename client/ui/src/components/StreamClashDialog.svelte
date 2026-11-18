<script lang="ts">
  // Asks before a stream action ends another stream of yours (#339): watching elsewhere while
  // you're live, or going live while you watch elsewhere as a ghost. Cancel has the focus first.
  import { modal } from '../lib/actions'
  import { displayName } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { streamApi, type StreamClash } from '../lib/stream/stream.svelte'

  let { clash }: { clash: StreamClash } = $props()

  const channel = (id: string) => app.state.channels[id]?.name ?? 'another channel'
  const live = $derived(channel(clash.liveIn))
  const there = $derived(channel(clash.channelId))
  const name = $derived(displayName(app.state, app.state.channels[clash.channelId]?.server_id ?? null, clash.streamer))
  const title = $derived(clash.kind === 'watch' ? 'Stop streaming?' : 'Stop watching?')
  const go = $derived(clash.kind === 'watch' ? 'Stop & watch' : 'Stop & go live')

  const cancel = () => void streamApi.resolveClash(false)
  const confirm = () => void streamApi.resolveClash(true)
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">{title}</h1>
    {#if clash.kind === 'watch'}
      <p>You're live in {live}. Stop streaming to watch {name} in {there}?</p>
    {:else}
      <p>You're watching {name} in {there}. Stop watching to go live in {live}?</p>
    {/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-primary" onclick={confirm}>{go}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(400px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
