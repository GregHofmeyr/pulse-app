<script lang="ts">
  // A wheel's options (#158), shared by the wheel popover and Settings › Wheels: add one (pasting `a, b, c` adds
  // several), remove one, Shuffle, Clear all. Each option shows its slice's colour. `onChange` gets the new list;
  // the owner saves it (a saved wheel) or keeps it (a quick wheel). A saved wheel also has a weight per option and a
  // palette (#259): `weighted` shows each option's weight (1 to 10, its share of the wheel and of the draw), and
  // `onPalette` the palette picker.
  import Icon from './Icon.svelte'
  import { addOptions, cleanWeights, shuffled, weightsFor, WHEEL_LIMITS } from '../lib/wheel'
  import { PALETTES, paletteOf, sliceStyle } from '../lib/wheelColours'

  let { options, weights = null, weighted = false, palette = null, onPalette, onChange, disabled = false, autofocus = false, draft = $bindable('') }: {
    options: string[]
    /** Each option's weight (#259); none: all alike. */
    weights?: readonly number[] | null
    /** Show and edit each option's weight (a saved wheel). */
    weighted?: boolean
    /** The wheel's palette (#259); none is the neon spectrum. */
    palette?: string | null
    /** Shows the palette picker (a saved wheel). */
    onPalette?: (palette: string) => void
    /** The new options, and their weights (none: all alike) kept beside them. */
    onChange: (options: string[], weights?: number[]) => void
    disabled?: boolean
    /** Focus the add field on mount (a new or cleared wheel). */
    autofocus?: boolean
    /** What's typed in the add field and not yet added (the owner may ask whether it's unsaved, or empty it). */
    draft?: string
  } = $props()

  let note = $state('')
  let field: HTMLInputElement | undefined = $state()
  const ws = $derived(weightsFor(options.length, weights))
  const total = $derived(ws.reduce((a, b) => a + b, 0))
  const chosen = $derived(paletteOf(palette))

  /** Options and their weights changed together, the weights kept beside their options. */
  const change = (next: string[], w: number[]) => onChange(next, cleanWeights(w))

  function setWeight(i: number, raw: number) {
    if (!Number.isFinite(raw)) return
    const v = Math.min(WHEEL_LIMITS.maxWeight, Math.max(1, Math.round(raw)))
    if (v === ws[i]) return
    change(options, ws.map((x, j) => (j === i ? v : x)))
  }

  $effect(() => {
    if (autofocus) field?.focus()
  })

  function add(e: SubmitEvent) {
    e.preventDefault()
    if (!draft.trim()) return
    const r = addOptions(options, draft)
    note = r.full ? `A wheel has at most ${WHEEL_LIMITS.maxOptions} options.` : ''
    draft = ''
    // new options weigh 1
    if (r.options.length !== options.length) change(r.options, [...ws, ...Array(r.options.length - options.length).fill(1)])
  }

  function remove(i: number) {
    note = ''
    change(options.filter((_, j) => j !== i), ws.filter((_, j) => j !== i))
  }

  function shuffle() {
    const pairs = shuffled(options.map((o, i) => [o, ws[i]] as const))
    change(pairs.map((p) => p[0]), pairs.map((p) => p[1]))
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
          <span class="swatch" style:background={sliceStyle(i, options.length, palette).fill}></span>
          <span class="text">{o}</span>
          {#if weighted}
            <label class="weight" data-tip="{Math.round((ws[i] / total) * 100)}% of the wheel">
              <span class="times" aria-hidden="true">×</span>
              <input type="number" class="input input-sm" min="1" max={WHEEL_LIMITS.maxWeight} step="1" value={ws[i]} {disabled}
                aria-label="Weight of {o}" onchange={(e) => setWeight(i, Number(e.currentTarget.value))} />
            </label>
          {/if}
          <button type="button" class="icon-btn icon-btn-xs" aria-label="Remove {o}" data-tip="Remove" {disabled} onclick={() => remove(i)}><Icon name="x" size={14} /></button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="tools">
    <button type="button" class="btn btn-secondary" disabled={disabled || options.length < 2} onclick={shuffle}>Shuffle</button>
    <button type="button" class="btn btn-secondary" disabled={disabled || options.length === 0} onclick={clearAll}>Clear all</button>
    <span class="count">{options.length} / {WHEEL_LIMITS.maxOptions}</span>
  </div>
  {#if onPalette}
    <div class="palettes" role="radiogroup" aria-label="Colours">
      <span class="menu-label">Colours</span>
      {#each PALETTES as p (p.id)}
        <button type="button" role="radio" class="toggle-chip" class:on={chosen === p.id} aria-checked={chosen === p.id} {disabled}
          onclick={() => chosen !== p.id && onPalette(p.id)}>
          <span class="dots" aria-hidden="true">
            {#each [0, 1, 2, 3] as k (k)}<span class="dot" style:background={sliceStyle(k, 4, p.id).fill}></span>{/each}
          </span>
          {p.label}
        </button>
      {/each}
    </div>
  {/if}
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
  .weight { display: flex; align-items: center; gap: 2px; color: var(--text-3); font-size: 12px; }
  .weight input { width: 46px; padding-right: 2px; text-align: right; font-variant-numeric: tabular-nums; }
  .palettes { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .palettes .menu-label { margin-right: 2px; }
  .dots { display: inline-flex; gap: 1px; }
  .dot { width: 6px; height: 10px; border-radius: 1px; }
  .count { margin-left: auto; color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
