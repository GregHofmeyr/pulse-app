<script lang="ts">
  import { toasts, watchStream, removeToast } from '../lib/stream/viewer.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { voice } from '../lib/voice.svelte'
  import { stream } from '../lib/stream/stream.svelte'

  // Neutral one-liners from the stream store ("<name> stopped streaming", watch errors) fade after a few seconds.
  const MESSAGE_MS = 4_000
  const message = $derived(stream.watchError || stream.notice)
  const windows = isWindows()
  $effect(() => {
    if (!message) return
    const t = setTimeout(() => {
      stream.watchError = ''
      stream.notice = ''
    }, MESSAGE_MS)
    return () => clearTimeout(t)
  })
</script>

<div class="stack" aria-live="polite">
  {#each toasts.list as t (t.channelId + t.streamer)}
    <div class="toast" role="status">
      <span><strong>{t.name}</strong> is live</span>
      {#if !windows}
        <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the reason -->
        <button type="button" class="watch off" aria-disabled="true" data-tip={watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId: t.channelId })}>Watch</button>
      {:else}
        <!-- out of that call (you've left since), this joins it again first -->
        <button type="button" class="watch" onclick={() => watchStream(t.channelId, t.streamer)}>{voice.channelId === t.channelId ? 'Watch' : 'Join & watch'}</button>
      {/if}
      <button type="button" class="x" aria-label="Dismiss" onclick={() => removeToast(t.channelId, t.streamer)}>×</button>
    </div>
  {/each}
  {#if message}<div class="toast" role="status" class:err={!!stream.watchError}><span>{message}</span></div>{/if}
</div>

<style>
  .stack { position: fixed; right: 18px; bottom: 18px; z-index: 40; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
  .toast { display: flex; align-items: center; gap: 12px; padding: 10px 12px 10px 14px; background: #2e3037; border: 1px solid #3d4048; border-radius: 12px; box-shadow: 0 12px 32px rgba(8, 9, 12, .5); font-size: 13px; }
  .toast.err { border-color: var(--danger); }
  .watch { height: 28px; padding: 0 12px; border: 0; border-radius: 8px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .watch.off { opacity: .5; cursor: not-allowed; }
  .x { border: 0; background: transparent; color: var(--text-3); font-size: 16px; padding: 0 2px; }
</style>
