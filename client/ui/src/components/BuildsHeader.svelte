<script lang="ts">
  // A builds channel's header (#617): the selected game's name (or "All games"), the count, a search box, the Gallery /
  // List switch and, rightmost, the panel toggle.
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import type { BuildView } from '../lib/buildPrefs'

  let { title, count, q, onQ, view, onView, panel, onPanel }: {
    title: string
    /** Builds in view (null until counted). */
    count: number | null
    q: string
    /** The search words, 250 ms after typing stops. */
    onQ: (q: string) => void
    view: BuildView
    onView: (v: BuildView) => void
    panel?: boolean
    onPanel?: () => void
  } = $props()

  const SEARCH_DEBOUNCE_MS = 250
  let typed = $state(untrack(() => q))
  let timer: ReturnType<typeof setTimeout> | undefined
  function input() {
    clearTimeout(timer)
    timer = setTimeout(() => onQ(typed.trim()), SEARCH_DEBOUNCE_MS)
  }
  $effect(() => () => clearTimeout(timer))
</script>

<div class="builds-head">
  <span class="ico"><Icon name="hammer" size={18} /></span>
  <h2>{title}</h2>
  {#if count !== null}<span class="count">{count === 1 ? '1 build' : `${count} builds`}</span>{/if}
  <span class="grow"></span>
  <label class="search">
    <Icon name="search" size={14} />
    <input class="input input-sm" type="search" placeholder="Search builds" aria-label="Search builds" maxlength="200" bind:value={typed} oninput={input} />
  </label>
  <Segmented label="View" size="sm" options={[{ value: 'gallery', label: 'Gallery' }, { value: 'list', label: 'List' }]} value={view} onchange={onView} />
  {#if onPanel}
    <button type="button" class="icon-btn hbtn" class:on={panel} aria-label={panel ? 'Hide games panel' : 'Show games panel'} data-tip={panel ? 'Hide panel' : 'Show panel'}
      aria-pressed={panel} onclick={onPanel}><Icon name="panelRight" size={17} /></button>
  {/if}
</div>

<style>
  .builds-head { position: relative; min-height: 52px; flex-shrink: 0; padding: 8px 18px; box-sizing: border-box; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .ico { display: flex; }
  h2 { margin: 0; font-size: 15px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 280px; }
  .count { font-size: 13px; font-variant-numeric: tabular-nums; }
  .grow { flex: 1; }
  .search { position: relative; display: flex; align-items: center; }
  .search :global(svg) { position: absolute; left: 9px; pointer-events: none; }
  .search input { width: 200px; padding-left: 28px; }
  .hbtn.on { background: var(--bg-3); color: var(--text); }
</style>
