<script lang="ts">
  // A wheel's options (#158), shared by the wheel popover and Settings › Wheels: add one (pasting `a, b, c` adds
  // several), remove one, Shuffle, Clear all. Each option shows its slice's colour. `onChange` gets the new list;
  // the owner saves it (a saved wheel) or keeps it (a quick wheel).
  import Icon from './Icon.svelte'
  import { addOptions, shuffled, WHEEL_LIMITS } from '../lib/wheel'
  import { sliceColour } from '../lib/wheelColours'

  let { options, onChange, disabled = false, autofocus = false }: {
    options: string[]
    onChange: (options: string[]) => void
    disabled?: boolean
    /** Focus the add field on mount (a new or cleared wheel). */
    autofocus?: boolean
  } = $props()

  let draft = $state('')
  let note = $state('')
  let field: HTMLInputElement | undefined = $state()

  $effect(() => {
    if (autofocus) field?.focus()
  })

  function add(e: SubmitEvent) {
    e.preventDefault()
    if (!draft.trim()) return
    const r = addOptions(options, draft)
    note = r.full ? `A wheel has at most ${WHEEL_LIMITS.maxOptions} options.` : ''
    draft = ''
    if (r.options.length !== options.length) onChange(r.options)
  }

  function remove(i: number) {
    note = ''
    onChange(options.filter((_, j) => j !== i))
  }

  function clearAll() {
    note = ''
    onChange([])
    field?.focus()
  }

  /** Focus the add field (after Clear all elsewhere, or a new wheel). */
  export function focus() {
    field?.focus()
  }
</script>

<div class="editor">
  <form class="add" onsubmit={add}>
    <input bind:this={field} class="input input-sm" bind:value={draft} {disabled} maxlength="2000"
      placeholder="Add an option (or paste a, b, c)" aria-label="Add an option" />
    <button type="submit" class="btn btn-secondary" disabled={disabled || !draft.trim()}>Add</button>
  </form>
  {#if note}<p class="note" role="status">{note}</p>{/if}
  {#if options.length === 0}
    <p class="empty">No options yet. Add a few above.</p>
  {:else}
    <ul class="options" aria-label="Options">
      {#each options as o, i (i + '\u0000' + o)}
        <li>
          <span class="swatch" style:background={sliceColour(i, options.length)}></span>
          <span class="text">{o}</span>
          <button type="button" class="icon-btn icon-btn-xs" aria-label="Remove {o}" data-tip="Remove" {disabled} onclick={() => remove(i)}><Icon name="x" size={14} /></button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="tools">
    <button type="button" class="btn btn-secondary" disabled={disabled || options.length < 2} onclick={() => onChange(shuffled(options))}>Shuffle</button>
    <button type="button" class="btn btn-secondary" disabled={disabled || options.length === 0} onclick={clearAll}>Clear all</button>
    <span class="count">{options.length} / {WHEEL_LIMITS.maxOptions}</span>
  </div>
</div>

<style>
  .editor { display: flex; flex-direction: column; gap: 8px; min-width: 0; min-height: 0; }
  .add { display: flex; gap: 6px; }
  .add .input { flex: 1; min-width: 0; }
  .note { margin: 0; color: var(--text-3); font-size: 12px; }
  .empty { margin: 0; padding: 6px 8px; color: var(--text-3); font-size: 13px; }
  .options { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; min-height: 0; flex: 1; }
  li { display: flex; align-items: center; gap: 8px; min-height: 28px; padding: 0 2px 0 8px; border-radius: var(--radius-sm); font-size: 13px; color: var(--text); }
  li:hover { background: var(--bg-3); }
  li .icon-btn:hover:not(:disabled) { background: var(--bg-4); color: var(--danger-text); }
  .swatch { width: 10px; height: 10px; border-radius: var(--radius-xs); flex-shrink: 0; }
  .text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tools { display: flex; align-items: center; gap: 6px; }
  .count { margin-left: auto; color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
