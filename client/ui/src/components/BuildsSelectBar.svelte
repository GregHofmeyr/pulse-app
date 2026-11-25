<script lang="ts">
  // The selection bar of the builds browse view's multi-select (#617): Cancel, "N selected" (announced as it
  // changes), how many of them aren't yours, then the actions and Clear. It lies over the header, in exactly its box,
  // so nothing below moves when selecting starts or ends; one line at any width: narrow, the actions keep only their
  // icons, with their names still read out; narrower, all but the first action and Clear move into a "More" menu;
  // narrowest, every action does.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'
  import type { SelectBarAction } from '../lib/buildSelect'

  let { count, notYours = 0, actions, busy = false, onClear, onCancel }: {
    count: number
    /** Selected builds you may change in no way: actions skip them. */
    notYours?: number
    /** In order; adding one (Favourite) is one more entry. */
    actions: SelectBarAction[]
    busy?: boolean
    onClear: () => void
    onCancel: () => void
  } = $props()

  let moreOpen = $state(false)
  let moreBtn = $state<HTMLButtonElement>()
  let menuEl = $state<HTMLElement>()
  const menuItems = () => (menuEl ? [...menuEl.querySelectorAll<HTMLButtonElement>('[role=menuitem]:not(:disabled)')] : [])
  async function openMore() {
    moreOpen = !moreOpen
    if (!moreOpen) return
    await tick()
    menuItems()[0]?.focus()
  }
  function closeMore(restoreFocus: boolean) {
    if (!moreOpen) return
    moreOpen = false
    if (restoreFocus) moreBtn?.focus()
  }
  function pickMore(run: () => void) {
    closeMore(true)
    run()
  }
  function menuKey(e: KeyboardEvent) {
    const l = menuItems()
    const i = l.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'Escape') {
      // handled here: selecting goes on
      e.preventDefault()
      e.stopPropagation()
      closeMore(true)
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (l.length) l[(i + (e.key === 'ArrowDown' ? 1 : -1) + l.length) % l.length].focus()
    } else if (e.key === 'Tab') closeMore(false)
  }
</script>

<div class="select-bar" role="toolbar" aria-label="Selected builds">
  <button type="button" class="icon-btn" aria-label="Cancel" data-tip="Cancel (Esc)" onclick={onCancel}><Icon name="x" size={16} /></button>
  <span class="count" role="status" aria-live="polite" data-testid="builds-selection-status">{count} selected</span>
  {#if notYours}<span class="note">· {notYours} not yours</span>{/if}
  <span class="grow"></span>
  {#each actions as a, i (a.key)}
    <button type="button" class="btn btn-secondary act" class:extra={i > 0} class:danger={a.danger} aria-label={a.label} data-tip={a.label} disabled={busy || a.disabled} onclick={a.run}>
      <Icon name={a.icon} size={14} /><span class="label" aria-hidden="true">{a.label}</span>
    </button>
  {/each}
  <button type="button" class="btn act clear" aria-label="Clear" disabled={busy || count === 0} onclick={onClear}><span class="label-always">Clear</span></button>
  <button type="button" class="icon-btn more" bind:this={moreBtn} aria-label="More actions" data-tip="More actions" aria-haspopup="menu" aria-expanded={moreOpen} data-menu-toggle onclick={() => void openMore()}><Icon name="dots" size={16} /></button>
</div>
{#if moreOpen}
  <!-- outside the bar (which clips), under its right end -->
  <div class="select-more" role="menu" aria-label="More actions" tabindex="-1" bind:this={menuEl} onkeydown={menuKey} use:clickOutside={(e) => closeMore(e.type === 'keydown')}>
    {#each actions as a (a.key)}
      <button type="button" role="menuitem" class="item" class:danger={a.danger} disabled={busy || a.disabled} onclick={() => pickMore(a.run)}><Icon name={a.icon} size={14} /><span class="item-label">{a.label}</span></button>
    {/each}
    <button type="button" role="menuitem" class="item" disabled={busy || count === 0} onclick={() => pickMore(onClear)}><span class="item-label">Clear</span></button>
  </div>
{/if}

<style>
  /* exactly over the header's box (its parent is the header's wrapper), above it */
  .select-bar { position: absolute; inset: 0; z-index: 3; container-type: inline-size; display: flex; align-items: center; gap: 6px; min-width: 0; padding: 0 12px; box-sizing: border-box; overflow: hidden; background: var(--bg-2); border-bottom: 1px solid var(--focus-ring); color: var(--text-2); }
  .count { flex-shrink: 1; min-width: 0; font-size: 14px; font-weight: 600; color: var(--text); font-variant-numeric: tabular-nums; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .note { flex-shrink: 2; min-width: 0; font-size: 13px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .grow { flex: 1; min-width: 0; }
  .act { flex-shrink: 0; }
  .act.danger:not(:disabled) { color: var(--danger-text); }
  .act:disabled { color: var(--text-3); cursor: default; }
  .clear { color: var(--text-3); padding: 0 8px; }
  .more { display: none; flex-shrink: 0; }
  /* narrow: the actions keep their icons (their names are their labels), the note goes */
  @container (max-width: 760px) {
    .label { display: none; }
    .act { padding: 0 9px; }
    .note { display: none; }
  }
  /* narrower: the first action stays; the others and Clear are in More */
  @container (max-width: 420px) {
    .extra, .clear { display: none; }
    .more { display: inline-flex; }
  }
  /* narrowest: Cancel, the count (cut short with an ellipsis) and More */
  @container (max-width: 240px) {
    .act { display: none; }
  }
  /* More's menu: under the bar's right end (the header's wrapper is its box), never wider than the view */
  .select-more { position: absolute; top: 100%; right: 8px; z-index: 4; display: flex; flex-direction: column; min-width: 160px; max-width: calc(100% - 16px); padding: 4px; box-sizing: border-box; background: var(--bg-2); border: 1px solid var(--bg-3); border-radius: var(--radius-sm); box-shadow: var(--shadow-2); }
  .item { display: flex; align-items: center; gap: 8px; min-width: 0; padding: 6px 10px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text); font: inherit; font-size: 13px; text-align: left; cursor: pointer; }
  .item:hover:not(:disabled), .item:focus-visible { background: var(--bg-3); }
  .item:disabled { color: var(--text-3); cursor: default; }
  .item.danger:not(:disabled) { color: var(--danger-text); }
  .item :global(svg) { flex-shrink: 0; }
  .item-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
