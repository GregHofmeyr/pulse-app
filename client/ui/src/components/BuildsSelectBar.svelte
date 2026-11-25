<script lang="ts">
  // The selection bar of the builds browse view's multi-select (#617): Cancel, "N selected" (announced as it
  // changes), how many of them aren't yours, then the actions and Clear. It lies over the header, in exactly its box,
  // so nothing below moves when selecting starts or ends; one line at any width (narrow, the actions keep only their
  // icons, with their names still read out).
  import Icon from './Icon.svelte'
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
</script>

<div class="select-bar" role="toolbar" aria-label="Selected builds">
  <button type="button" class="icon-btn" aria-label="Cancel" data-tip="Cancel (Esc)" onclick={onCancel}><Icon name="x" size={16} /></button>
  <span class="count" role="status" aria-live="polite" data-testid="builds-selection-status">{count} selected</span>
  {#if notYours}<span class="note">· {notYours} not yours</span>{/if}
  <span class="grow"></span>
  {#each actions as a (a.key)}
    <button type="button" class="btn btn-secondary act" class:danger={a.danger} aria-label={a.label} data-tip={a.label} disabled={busy || a.disabled} onclick={a.run}>
      <Icon name={a.icon} size={14} /><span class="label" aria-hidden="true">{a.label}</span>
    </button>
  {/each}
  <button type="button" class="btn act clear" aria-label="Clear" disabled={busy || count === 0} onclick={onClear}><span class="label-always">Clear</span></button>
</div>

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
  /* narrow: the actions keep their icons (their names are their labels), the note goes */
  @container (max-width: 520px) {
    .label { display: none; }
    .act { padding: 0 9px; }
    .note { display: none; }
  }
</style>
