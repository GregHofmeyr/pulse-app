<script lang="ts">
  // The download icon on a clip's video (#204) or a quoted image (#206), Discord-style: top right, shown while the
  // pointer is over it (the parent's :hover rule) or when it has keyboard focus. `beside`: left of the wall card's
  // full-screen button. With a download `key` (#205) it stays shown while that file saves: a ring round a stop
  // cross (click again to cancel), then a check for a moment.
  import Icon from './Icon.svelte'
  import ProgressRing from './ProgressRing.svelte'
  import { downloads } from '../lib/downloads.svelte'

  let { onclick, beside = false, label = 'Download clip', tip = 'Download', key }: {
    onclick: (e: MouseEvent) => void
    beside?: boolean
    label?: string
    tip?: string
    /** The download this icon starts (`clipKey`, `imageKey`, `fileKey`): shows its progress. */
    key?: string
  } = $props()

  const view = $derived(key ? downloads.view(key) : null)
  const percent = $derived(view?.phase === 'downloading' && view.fraction !== null ? Math.round(view.fraction * 100) : null)
  const aria = $derived(
    view?.phase === 'downloading' ? (percent === null ? 'Cancel download' : `Cancel download, ${percent}%`)
    : view?.phase === 'saved' ? 'Saved'
    : label,
  )
  const hint = $derived(view?.phase === 'downloading' ? 'Cancel download' : view?.phase === 'saved' ? 'Saved' : tip)
</script>

<button type="button" class="clip-dl" class:beside class:active={!!view} aria-label={aria} data-tip={hint} {onclick}>
  {#if view?.phase === 'downloading'}
    <ProgressRing fraction={view.fraction} />
    <Icon name="x" size={12} />
  {:else if view?.phase === 'saved'}
    <Icon name="check" size={16} />
  {:else}
    <Icon name="download" size={16} />
  {/if}
</button>

<style>
  /* the wall card's full-screen button's look (ClipCard .fs) */
  .clip-dl { position: absolute; z-index: 1; right: 8px; top: 8px; width: 28px; height: 28px; border: 0; border-radius: var(--radius-sm); background: rgba(0, 0, 0, .6); color: #fff; display: grid; place-items: center; cursor: pointer; opacity: 0; transition: opacity var(--dur) var(--ease), background-color var(--dur) var(--ease); --ring-track: rgba(255, 255, 255, .25); }
  .clip-dl:hover { background: rgba(0, 0, 0, .75); }
  .clip-dl:focus-visible { opacity: 1; }
  /* downloading or just saved: shown without the pointer, so it can be watched and stopped */
  .clip-dl.active { opacity: 1; }
  .clip-dl.beside { right: 40px; }
</style>
