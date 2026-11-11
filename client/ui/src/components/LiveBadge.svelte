<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { isLive } from '../lib/selectors'
  import LivePopover from './LivePopover.svelte'

  /** `marker`: a still LIVE mark inside something that's already a button (a conversation's row, #286): no popover,
   *  just this text as its tooltip and label. */
  let { channelId, userId, marker }: { channelId: string; userId: string; marker?: string } = $props()

  const HOVER_MS = 300
  const live = $derived(isLive(app.state, channelId, userId))
  let anchor = $state<HTMLElement>()
  let open = $state(false)
  let timer: ReturnType<typeof setTimeout> | undefined

  const hoverOpen = () => {
    clearTimeout(timer)
    timer = setTimeout(() => (open = true), HOVER_MS)
  }
  const hoverClose = () => {
    clearTimeout(timer)
    timer = setTimeout(() => (open = false), HOVER_MS)
  }
  const toggle = () => {
    clearTimeout(timer)
    open = !open
  }
  // the stream ended: the popover goes with the badge
  $effect(() => {
    if (!live) open = false
  })
  $effect(() => () => clearTimeout(timer))
</script>

{#if live && marker}
  <span class="live" role="img" aria-label={marker} data-tip={marker}>LIVE</span>
{:else if live}
  <span class="wrap" role="presentation" onmouseenter={hoverOpen} onmouseleave={hoverClose}>
    <button bind:this={anchor} type="button" class="live" aria-haspopup="dialog" aria-expanded={open} onclick={toggle}>LIVE</button>
    {#if open && anchor}
      <LivePopover {channelId} {userId} {anchor} onClose={() => (open = false)} />
    {/if}
  </span>
{/if}

<style>
  .wrap { display: inline-flex; }
  .live { height: 16px; padding: 0 6px; border: 0; border-radius: 4px; background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 800; letter-spacing: .06em; line-height: 16px; cursor: pointer; }
  span.live { display: inline-block; flex-shrink: 0; cursor: inherit; }
</style>
