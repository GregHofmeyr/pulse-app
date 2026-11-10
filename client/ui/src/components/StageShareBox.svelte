<script lang="ts">
  // Share a Spotify link to the stage (#234): a popover under the stage header's Spotify button, as the chat's pins
  // are under theirs. The server checks the link (through the embed resolver) and says why when it won't take it.
  import { clickOutside } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'

  let { channelId, onClose }: { channelId: string; onClose: () => void } = $props()

  let url = $state('')
  let busy = $state(false)
  let error = $state('')
  let input = $state<HTMLInputElement>()
  $effect(() => input?.focus())

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    const link = url.trim()
    if (!link || busy) return
    busy = true
    error = ''
    try {
      await api.shareToStage(channelId, link)
      onClose()
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions (Esc closes it, as on every popover) -->
<div class="share-box menu-surface pop-in" role="dialog" aria-label="Share Spotify to the stage" tabindex="-1" use:clickOutside={onClose}
  onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose() } }}>
  <h2>Share to the stage</h2>
  <p class="hint">A Spotify Jam invite, or a song, album, playlist or podcast link. Everyone in the call sees it.</p>
  <form class="row" onsubmit={submit}>
    <input bind:this={input} class="input input-sm" type="text" aria-label="Spotify link" placeholder="Paste a Spotify link" bind:value={url} maxlength="2000" />
    <button class="btn btn-primary sm" disabled={!url.trim() || busy}>Share</button>
  </form>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  /* the pins popover's place and size, under the header's buttons */
  .share-box { position: absolute; top: calc(100% + 6px); right: 12px; z-index: 30; width: min(400px, calc(100% - 24px)); box-sizing: border-box; padding: 14px; gap: 8px; color: var(--text); }
  h2 { margin: 0; font-size: 15px; }
  .hint { margin: 0; color: var(--text-3); font-size: 13px; }
  .row { display: flex; gap: 6px; }
  .row .input { flex: 1; min-width: 0; }
  .sm { height: 32px; padding: 0 12px; font-size: 13px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
