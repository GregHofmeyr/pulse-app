<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { liveIn } from '../lib/selectors'
  import { voice } from '../lib/voice.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { watchStream } from '../lib/stream/viewer.svelte'
  import { streamCmd } from '../lib/tauri'

  let { channelId, userId, anchor, onClose }: { channelId: string; userId: string; anchor: HTMLElement; onClose: () => void } = $props()

  const PREVIEW_EVERY_MS = 15_000
  const info = $derived(liveIn(app.state, channelId)[userId])
  const mine = $derived(userId === app.state.me?.id)
  const windows = isWindows()
  const inChannel = $derived(voice.channelId === channelId)
  // only Linux blocks watching here: out of the call, the button joins first
  const reason = $derived(windows ? null : watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId }))
  let preview = $state<string | null>(null)
  let pop = $state<HTMLElement>()

  // Fixed, from the badge's position: sidebars and grids scroll and would clip an absolute one.
  const rect = untrack(() => anchor).getBoundingClientRect()
  const left = Math.min(rect.right + 8, window.innerWidth - 300)
  const top = Math.max(8, Math.min(rect.top, window.innerHeight - 280))

  onMount(() => {
    let gone = false
    const fetchPreview = () =>
      streamCmd
        .previewGet(channelId, userId)
        .then((p) => !gone && (preview = p))
        .catch(() => {})
    void fetchPreview()
    const every = setInterval(fetchPreview, PREVIEW_EVERY_MS)
    return () => {
      gone = true
      clearInterval(every)
    }
  })

  function watch() {
    if (reason) return
    // out of the call, this joins first
    watchStream(channelId, userId)
    onClose()
  }
</script>

<svelte:window
  onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); onClose() } }}
  onpointerdown={(e) => !pop?.contains(e.target as Node) && !anchor.contains(e.target as Node) && onClose()} />

{#if info}
  <div bind:this={pop} class="pop" role="dialog" aria-label="Live stream" style:left="{left}px" style:top="{top}px">
    {#if preview}<img src={preview} alt="Stream preview" />{:else}<div class="none">No preview yet</div>{/if}
    <div class="meta">
      <span>{info.settings.height}p{info.settings.fps}</span>
      <span>{info.viewers.length} watching</span>
    </div>
    {#if !mine}
      <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the reason -->
      <button type="button" class="watch" class:off={!!reason} aria-disabled={!!reason} data-tip={reason ?? undefined} onclick={watch}>{inChannel || reason ? 'Watch' : 'Join & watch'}</button>
      {#if reason}<small>{reason}</small>{/if}
    {/if}
  </div>
{/if}

<style>
  .pop { position: fixed; z-index: 30; width: 280px; padding: 10px; background: #2e3037; border: 1px solid #3d4048; border-radius: 14px; box-shadow: 0 18px 48px rgba(8, 9, 12, .5); display: flex; flex-direction: column; gap: 8px; font-size: 13px; }
  img, .none { width: 100%; aspect-ratio: 16 / 9; border-radius: 8px; background: var(--bg-1); object-fit: cover; }
  .none { display: grid; place-items: center; background: var(--bg-4); color: var(--text-2); }
  .meta { display: flex; justify-content: space-between; color: var(--text-2); }
  .watch { height: 34px; border: 0; border-radius: 10px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .watch.off { opacity: .5; cursor: not-allowed; }
  small { color: var(--text-3); text-align: center; }
</style>
