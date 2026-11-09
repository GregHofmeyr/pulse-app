<script lang="ts" module>
  // A quick wheel's options per chat, kept while the app runs (closing the popover doesn't lose them).
  const quickDrafts = new Map<string, string[]>()
  /** For tests: forget every chat's quick wheel. */
  export const forgetQuickWheels = () => quickDrafts.clear()
</script>

<script lang="ts">
  // The wheel popover above the composer (#158): pick a saved wheel (Server or Just me) or type a quick one, then
  // Spin. A server or quick wheel's spin goes to the chat, where everyone watches it land; "Spin quietly" (#270,
  // remembered per device) leaves no line behind. A Just me wheel spins here, for you alone (#270). Saved wheels
  // save as you edit them; a quick wheel can be kept with Save as wheel. In a DM or group there is no server: Just
  // me wheels only.
  import { onDestroy, onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import Wheel from './Wheel.svelte'
  import WheelEditor from './WheelEditor.svelte'
  import SaveWheelForm from './SaveWheelForm.svelte'
  import { clickOutside } from '../lib/actions'
  import { popoverStyle } from '../lib/reactions'
  import { errorText, wheelCmd } from '../lib/tauri'
  import {
    loadMyWheels, loadServerWheels, saveWheel, savesSettled, setSpinQuietly, spinQuietly, spinWheel, upsertWheel, wheelPrefs, wheels,
  } from '../lib/wheels.svelte'
  import { drawSpin, WHEEL_LIMITS } from '../lib/wheel'
  import { sliceColour } from '../lib/wheelColours'
  import { quietNow, runSpin } from '../lib/wheelSpin'
  import type { Wheel as SavedWheel } from '../lib/protocol/Wheel'

  let { anchor, channelId, serverId, prefill = null, onClose }: {
    /** The composer's wheel button; it has `data-menu-toggle`, so a press on it toggles instead. */
    anchor: HTMLElement
    channelId: string
    /** The chat's server; null in DMs and groups (Just me wheels only). */
    serverId: string | null
    /** Options typed after `/wheel` (fewer than two): they start a quick wheel. */
    prefill?: string[] | null
    onClose: () => void
  } = $props()

  const SIZE = { width: 600, height: 480 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))

  let scope = $state<'server' | 'mine'>(untrack(() => (serverId ? 'server' : 'mine')))
  /** A saved wheel's id, or the quick wheel. */
  let selected = $state<string>('quick')
  let quick = $state<string[]>(untrack(() => (prefill?.length ? [...prefill] : (quickDrafts.get(channelId) ?? []))))
  let error = $state('')
  let busy = $state(false)
  let saving = $state(false)
  let nameDraft = $state('')
  let nameField: HTMLInputElement | undefined = $state()
  let editor: WheelEditor | undefined = $state()
  let addDraft = $state('')
  let root: HTMLDivElement | undefined = $state()

  // Escape in a field (#264) first leaves it, or empties the add field; a second Escape closes the popover. The
  // quick wheel outlives the popover and a saved one saves as you go, so there's nothing to ask about.
  function keydown(e: KeyboardEvent) {
    const t = e.target
    if (e.key !== 'Escape' || !(t instanceof HTMLInputElement) || t.closest('form.save')) return
    e.preventDefault()
    e.stopPropagation()
    if (t.getAttribute('aria-label') === 'Add an option' && addDraft) addDraft = ''
    else root?.focus() // out of the field, still in the popover (a name leaving it saves, as on blur)
  }

  $effect(() => {
    if (quick.length) quickDrafts.set(channelId, quick)
    else quickDrafts.delete(channelId)
  })

  onMount(() => {
    const loads: Promise<unknown>[] = [loadMyWheels()]
    if (serverId) loads.push(loadServerWheels(serverId))
    Promise.all(loads).catch((e) => (error = errorText(e)))
    void tick().then(() => editor?.focus())
  })

  const list = $derived(scope === 'server' && serverId ? (wheels.server[serverId] ?? []) : (wheels.mine ?? []))
  const current = $derived(selected === 'quick' ? null : (list.find((w) => w.id === selected) ?? null))
  const options = $derived(current ? current.options : quick)
  // the selected wheel was deleted (here, in Settings or by someone else): back to the quick wheel
  $effect(() => {
    if (selected !== 'quick' && !current && (scope === 'server' ? serverId && wheels.server[serverId] : wheels.mine)) selected = 'quick'
  })
  // a new wheel picked: its name in the field (a change to it by someone else doesn't clobber your typing)
  $effect(() => {
    void current?.id
    nameDraft = untrack(() => current?.name ?? '')
  })

  function pickScope(v: 'server' | 'mine') {
    scope = v
    selected = 'quick'
    saving = false
  }

  function pick(id: string) {
    selected = id
    saving = false
    error = ''
  }

  // a saved wheel saves each change, in order; what the server answers (or its event) is the truth
  async function save(w: SavedWheel, change: { name?: string; options?: string[] }) {
    error = ''
    error = await saveWheel(w, change)
  }

  function setOptions(next: string[]) {
    if (current) void save(current, { options: next })
    else quick = next
  }

  function rename() {
    const n = nameDraft.trim()
    if (current && n && n !== current.name) void save(current, { name: n })
    else nameDraft = current?.name ?? ''
  }

  async function newWheel() {
    error = ''
    const taken = new Set(list.map((w) => w.name.toLowerCase()))
    let name = 'New wheel'
    for (let i = 2; taken.has(name.toLowerCase()); i++) name = `New wheel ${i}`
    try {
      const w = await wheelCmd.create(scope === 'server' ? serverId : null, name, [])
      upsertWheel(w)
      selected = w.id
      await tick()
      nameField?.focus()
      nameField?.select()
    } catch (e) {
      error = errorText(e)
    }
  }

  async function spin() {
    if (isPrivate) return privateSpin(false)
    if (busy || options.length < WHEEL_LIMITS.minOptions) return
    busy = true
    error = ''
    try {
      // a change still saving goes first: the spin is of the wheel you see
      await savesSettled()
      const request = current ? { wheel_id: current.id } : { options: quick }
      await (wheelPrefs.quiet ? spinQuietly(channelId, request) : spinWheel(channelId, request))
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  // --- a Just me wheel's private spin (#270): drawn and shown here, sent nowhere. Remove winner & spin takes the
  // winner off this run of spins only (the saved wheel keeps it); picking, switching or editing starts afresh. ---
  let rotation = $state(0)
  let spinning = $state(false)
  let result = $state<{ winner: number; options: string[] } | null>(null)
  /** The private spins' options once a winner's been removed; null: the saved wheel's. */
  let privateOptions = $state<string[] | null>(null)
  let stopSpin: (() => void) | null = null
  onDestroy(() => stopSpin?.())
  $effect(() => {
    void current?.id
    void current?.options
    void scope
    untrack(() => {
      stopSpin?.()
      spinning = false
      result = null
      privateOptions = null
      rotation = 0
    })
  })

  function privateSpin(removeWinner: boolean) {
    if (!current || spinning) return
    let opts = privateOptions ?? current.options
    if (removeWinner && result) opts = result.options.filter((_, i) => i !== result!.winner)
    if (opts.length < WHEEL_LIMITS.minOptions) return
    error = ''
    privateOptions = opts
    result = null
    spinning = true
    const { winner, angle } = drawSpin(opts.length)
    // on from where the wheel stands, landing on the drawn angle
    const base = rotation % 360
    stopSpin = runSpin({
      angle: (((angle - base) % 360) + 360) % 360,
      count: opts.length,
      quiet: quietNow(),
      onRotate: (deg) => (rotation = base + deg),
      onLanded: () => {
        spinning = false
        result = { winner, options: opts }
      },
    })
  }

  function saved(w: SavedWheel) {
    saving = false
    scope = w.server_id ? 'server' : 'mine'
    selected = w.id
    quick = []
  }

  const isPrivate = $derived(current?.scope === 'personal')
  /** What the wheel shows: a private run's options (a winner removed), or the wheel being edited. */
  const shown = $derived(isPrivate && privateOptions ? privateOptions : options)
  const canSpin = $derived(shown.length >= WHEEL_LIMITS.minOptions && !busy && !spinning)
</script>

<div bind:this={root} class="wheel-popover pop-in" role="dialog" aria-label="Picker wheel" tabindex="-1" use:clickOutside={onClose} onkeydown={keydown} {style}>
  <div class="stage">
    <Wheel options={shown} {rotation} landed={result?.winner ?? null} size={210} onSpin={spin} spinDisabled={!canSpin} />
    <div class="what">
      <span class="name">{current?.name ?? 'Quick wheel'}</span>
      <small>{current ? (current.server_id ? 'Server wheel · everyone here sees it' : 'Just me · only you see its spins') : 'One-off, not saved'}</small>
    </div>
    {#if isPrivate}
      <p class="result" aria-live="polite">
        {#if result}
          <span class="swatch" style:background={sliceColour(result.winner, result.options.length)}></span> <strong>{result.options[result.winner]}</strong>
          {#if result.options.length > WHEEL_LIMITS.minOptions}<button type="button" class="link-btn" onclick={() => privateSpin(true)}>Remove winner &amp; spin</button>{/if}
        {/if}
      </p>
    {/if}
    <button type="button" class="btn btn-primary btn-lg spin" disabled={!canSpin} onclick={spin}>
      {shown.length < WHEEL_LIMITS.minOptions ? 'Add at least 2 options' : result ? 'Spin again' : 'Spin'}
    </button>
    {#if !isPrivate}
      <!-- the chat still watches it land; nothing is saved (#270) -->
      <label class="quietly" data-tip="Everyone here watches it land, but no line is saved">
        <input type="checkbox" checked={wheelPrefs.quiet} onchange={(e) => setSpinQuietly(e.currentTarget.checked)} /> Spin quietly
      </label>
    {/if}
    {#if !current && quick.length && !saving}
      <button type="button" class="btn btn-secondary" onclick={() => (saving = true)}>Save as wheel <Icon name="chevronRight" size={14} /></button>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
  <div class="side">
    {#if serverId}
      <div class="sec">
        <Segmented label="Wheels" options={[{ value: 'server', label: 'Server' }, { value: 'mine', label: 'Just me' }]} value={scope} onchange={pickScope} />
      </div>
    {/if}
    <div class="sec">
      <div class="saved-head">
        <span class="menu-label">Saved wheels</span>
        <button type="button" class="icon-btn icon-btn-xs" aria-label="New wheel" data-tip="New wheel" onclick={newWheel}><Icon name="plus" size={14} /></button>
      </div>
      <div class="wheels" role="listbox" aria-label="Wheels">
        <button type="button" class="wheel-row" role="option" aria-selected={selected === 'quick'} onclick={() => pick('quick')}>
          <span class="row-name">Quick wheel</span><span class="n">{quick.length}</span>
        </button>
        {#each list as w (w.id)}
          <button type="button" class="wheel-row" role="option" aria-selected={selected === w.id} onclick={() => pick(w.id)}>
            <span class="row-name">{w.name}</span><span class="n">{w.options.length}</span>
          </button>
        {/each}
      </div>
    </div>
    <div class="sec grow">
      {#if saving}
        <div class="menu-label">Save as wheel</div>
        <SaveWheelForm {serverId} options={quick} startScope={scope} onSaved={saved} onCancel={() => (saving = false)} />
      {:else}
        {#if current}
          <input bind:this={nameField} class="input input-sm" bind:value={nameDraft} maxlength={WHEEL_LIMITS.nameChars} aria-label="Wheel name"
            onblur={rename} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); rename() } }} />
        {:else}
          <div class="menu-label">Options</div>
        {/if}
        <WheelEditor bind:this={editor} bind:draft={addDraft} {options} onChange={setOptions} />
      {/if}
    </div>
  </div>
</div>

<style>
  /* the soundboard's frame: the menus' border, radius and shadow round a bg-2 panel */
  .wheel-popover { width: 600px; max-width: calc(100vw - 12px); height: 480px; max-height: calc(100vh - 12px); display: flex; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); color: var(--text); overflow: hidden; outline: none; --pop-origin: bottom left; }
  .stage { width: 250px; flex-shrink: 0; display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 18px 16px 14px; background: var(--bg-1); border-right: 1px solid var(--line); overflow-y: auto; }
  .what { display: flex; flex-direction: column; align-items: center; gap: 2px; text-align: center; min-width: 0; max-width: 100%; }
  .what .name { font-weight: 700; font-size: 15px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .what small { color: var(--text-3); font-size: 11px; }
  .spin { align-self: stretch; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; text-align: center; }
  /* the private spin's result, under the name: the winner's swatch, the winner, Remove winner & spin */
  .result { margin: 0; min-height: 20px; display: flex; align-items: center; justify-content: center; flex-wrap: wrap; gap: 4px 6px; font-size: 14px; color: var(--text); text-align: center; }
  .result .link-btn { font-size: 13px; }
  .swatch { display: inline-block; width: 10px; height: 10px; border-radius: var(--radius-xs); flex-shrink: 0; }
  .quietly { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text-2); cursor: pointer; }
  .quietly input { accent-color: var(--accent); margin: 0; }
  /* the side's sections (switch, saved wheels, the wheel's options) each in their own band, split by a line like
     Settings' sections and the poll composer's bands */
  .side { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .sec { display: flex; flex-direction: column; gap: 8px; padding: 12px; flex-shrink: 0; }
  .sec + .sec { border-top: 1px solid var(--line); }
  .sec.grow { flex: 1; flex-shrink: 1; min-height: 0; }
  .side .menu-label { padding: 2px 2px 0; }
  .saved-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .saved-head .icon-btn:hover:not(:disabled) { background: var(--bg-4); }
  .wheels { display: flex; flex-direction: column; gap: 1px; max-height: 112px; overflow-y: auto; flex-shrink: 0; }
  .wheel-row { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 0 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 13px; text-align: left; cursor: pointer; }
  .wheel-row:hover { background: var(--bg-3); color: var(--text); }
  .wheel-row[aria-selected='true'] { background: var(--accent-soft); color: var(--text); }
  .row-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n { color: var(--text-3); font-size: 11px; font-variant-numeric: tabular-nums; }
  .sec :global(.editor) { flex: 1; }
</style>
