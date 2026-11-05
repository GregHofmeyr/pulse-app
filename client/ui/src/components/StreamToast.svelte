<script lang="ts">
  import { toasts, watchModeFor, watchStream, removeToast } from '../lib/stream/viewer.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { voice } from '../lib/voice.svelte'
  import { stream } from '../lib/stream/stream.svelte'
  import Icon from './Icon.svelte'
  import { exitFade } from '../lib/exit'
  import { toastShown } from '../lib/toastPrefs.svelte'

  // Neutral one-liners from the stream store ("<name> stopped streaming", watch errors) fade after a few seconds.
  // The one-liners follow the "Stream updates" toast switch (#166); a watch error always shows.
  const MESSAGE_MS = 4_000
  const message = $derived(stream.watchError || (toastShown('stream') ? stream.notice : ''))
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
    <div class="toast fade-in" role="status" out:exitFade>
      <span><strong>{t.name}</strong> is live{#if t.where} in {t.where}{/if}</span>
      {#if !windows}
        <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the reason -->
        <button type="button" class="btn btn-primary watch off" aria-disabled="true" data-tip={watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId: t.channelId })}>Watch</button>
      {:else}
        <!-- out of that call (you've left since), this joins it again first -->
        <button type="button" class="btn btn-primary watch" onclick={() => watchStream(t.channelId, t.streamer)}>{watchModeFor(t.channelId, t.streamer) === 'join' ? 'Join & watch' : 'Watch'}</button>
      {/if}
      <button type="button" class="icon-btn icon-btn-xs" aria-label="Dismiss" onclick={() => removeToast(t.channelId, t.streamer)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
  {#if message}<div class="toast notice fade-in" role="status" class:err={!!stream.watchError} out:exitFade><span>{message}</span></div>{/if}
</div>

<style>
  .stack { position: fixed; right: 18px; bottom: 18px; z-index: 40; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
  /* the shared .toast (app.css) */
  .toast.err { border-color: var(--danger); }
</style>
