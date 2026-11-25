<script lang="ts">
  // The right panel's switcher (#638): one tab per view that could fill the column (lib/rightViews.svelte.ts picks
  // which shows). Tabs pattern: a tablist, roving tab order, arrows / Home / End move and pick. One row that never
  // wraps or spills: labels truncate with an ellipsis, and a narrow column shows icons only (named by aria-label and a
  // tooltip).
  import Icon, { type IconName } from './Icon.svelte'

  export type RightViewTab = { id: string; label: string; icon: IconName }

  let { views, current, width, panelId, onPick }: {
    views: RightViewTab[]
    current: string
    /** The column's width (px): below COMPACT_TAB px a tab, icons only. */
    width: number
    /** The tabpanel the tabs control. */
    panelId: string
    onPick: (id: string) => void
  } = $props()

  /** The least room a tab with its label takes before the row goes to icons only. */
  const COMPACT_TAB = 84
  const compact = $derived(width > 0 && width / Math.max(1, views.length) < COMPACT_TAB)
  let list: HTMLElement | undefined = $state()

  function keys(e: KeyboardEvent, i: number) {
    const n = views.length
    const to = e.key === 'ArrowRight' ? (i + 1) % n : e.key === 'ArrowLeft' ? (i - 1 + n) % n : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : -1
    if (to < 0) return
    e.preventDefault()
    onPick(views[to].id)
    list?.querySelectorAll<HTMLElement>('[role=tab]')[to]?.focus()
  }
</script>

<div bind:this={list} class="right-tabs" class:compact role="tablist" aria-label="Right panel">
  {#each views as v, i (v.id)}
    {@const on = v.id === current}
    <button type="button" class="tab" role="tab" id="right-tab-{v.id}" aria-selected={on} aria-controls={panelId} tabindex={on ? 0 : -1}
      aria-label={compact ? v.label : undefined} data-tip={compact ? v.label : undefined} onclick={() => onPick(v.id)} onkeydown={(e) => keys(e, i)}>
      <Icon name={v.icon} size={14} />
      {#if !compact}<span class="label">{v.label}</span>{/if}
    </button>
  {/each}
</div>

<style>
  /* contain: inline-size, so the tabs' labels never widen the column: the row takes the width the view sets */
  .right-tabs { flex-shrink: 0; display: flex; flex-wrap: nowrap; gap: 2px; padding: 4px; box-sizing: border-box; overflow: hidden; contain: inline-size; align-self: stretch; background: var(--bg-1); border-radius: var(--radius); }
  .tab { flex: 1 1 0; min-width: 0; height: 28px; display: inline-flex; align-items: center; justify-content: center; gap: 6px; padding: 0 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .tab:hover { color: var(--text); background: var(--bg-2); }
  .tab[aria-selected='true'] { color: var(--text); background: var(--bg-3); }
  .tab :global(svg) { flex-shrink: 0; }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compact .tab { padding: 0; }
</style>
