<script lang="ts" module>
  // A quick wheel's options per chat, kept while the app runs (closing the popover doesn't lose them).
  const quickDrafts = new Map<string, string[]>()
  /** For tests: forget every chat's quick wheel. */
  export const forgetQuickWheels = () => quickDrafts.clear()
</script>

<script lang="ts">
  // The wheel popover above the composer (#158): pick a saved wheel (Server or Just me) or type a quick one, then
  // Spin. The spin goes to the chat, where everyone watches it land. Saved wheels save as you edit them; a quick
  // wheel can be kept with Save as wheel. In a DM or group there is no server: Just me wheels only.
  import { onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import Wheel from './Wheel.svelte'
  import WheelEditor from './WheelEditor.svelte'
  import SaveWheelForm from './SaveWheelForm.svelte'
  import { clickOutside } from '../lib/actions'
  import { popoverStyle } from '../lib/reactions'
  import { errorText, wheelCmd } from '../lib/tauri'
  import { loadMyWheels, loadServerWheels, saveWheel, savesSettled, spinWheel, upsertWheel, wheels } from '../lib/wheels.svelte'
  import { WHEEL_LIMITS } from '../lib/wheel'
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
    if (busy || options.length < WHEEL_LIMITS.minOptions) return
    busy = true
    error = ''
    try {
      // a change still saving goes first: the spin is of the wheel you see
      await savesSettled()
      await spinWheel(channelId, current ? { wheel_id: current.id } : { options: quick })
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  function saved(w: SavedWheel) {
    saving = false
    scope = w.server_id ? 'server' : 'mine'
    selected = w.id
    quick = []
  }

  const canSpin = $derived(options.length >= WHEEL_LIMITS.minOptions && !busy)
</script>

<div class="wheel-popover pop-in" role="dialog" aria-label="Picker wheel" use:clickOutside={onClose} {style}>
  <div class="stage">
    <Wheel {options} size={210} onSpin={spin} spinDisabled={!canSpin} />
    <div class="what">
      <span class="name">{current?.name ?? 'Quick wheel'}</span>
      <small>{current ? (current.server_id ? 'Server wheel · everyone here sees it' : 'Just me · only you see it') : 'One-off, not saved'}</small>
    </div>
    <button type="button" class="btn btn-primary btn-lg spin" disabled={!canSpin} onclick={spin}>
      {options.length < WHEEL_LIMITS.minOptions ? 'Add at least 2 options' : 'Spin'}
    </button>
    {#if !current && quick.length && !saving}
      <button type="button" class="btn btn-secondary" onclick={() => (saving = true)}>Save as wheel <Icon name="chevronRight" size={14} /></button>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
  <div class="side">
    {#if serverId}
      <Segmented label="Wheels" options={[{ value: 'server', label: 'Server' }, { value: 'mine', label: 'Just me' }]} value={scope} onchange={pickScope} />
    {/if}
    <div class="menu-label">Saved wheels</div>
    <div class="wheels" role="listbox" aria-label="Wheels">
      <button type="button" class="wheel-row" role="option" aria-selected={selected === 'quick'} onclick={() => pick('quick')}>
        <span class="row-name">Quick wheel</span><span class="n">{quick.length}</span>
      </button>
      {#each list as w (w.id)}
        <button type="button" class="wheel-row" role="option" aria-selected={selected === w.id} onclick={() => pick(w.id)}>
          <span class="row-name">{w.name}</span><span class="n">{w.options.length}</span>
        </button>
      {/each}
      <button type="button" class="wheel-row new" onclick={newWheel}><Icon name="plus" size={14} /> New wheel</button>
    </div>
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
      <WheelEditor bind:this={editor} {options} onChange={setOptions} />
    {/if}
  </div>
</div>

<style>
  /* the soundboard's frame: the menus' border, radius and shadow round a bg-2 panel */
  .wheel-popover { width: 600px; max-width: calc(100vw - 12px); height: 480px; display: flex; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); color: var(--text); overflow: hidden; --pop-origin: bottom left; }
  .stage { width: 250px; flex-shrink: 0; display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 18px 16px 14px; background: var(--bg-1); border-right: 1px solid var(--line); }
  .what { display: flex; flex-direction: column; align-items: center; gap: 2px; text-align: center; min-width: 0; max-width: 100%; }
  .what .name { font-weight: 700; font-size: 15px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .what small { color: var(--text-3); font-size: 11px; }
  .spin { align-self: stretch; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; text-align: center; }
  .side { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; padding: 12px; min-height: 0; }
  .side .menu-label { padding: 2px 2px 0; }
  .wheels { display: flex; flex-direction: column; gap: 1px; max-height: 112px; overflow-y: auto; flex-shrink: 0; }
  .wheel-row { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 0 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 13px; text-align: left; cursor: pointer; }
  .wheel-row:hover { background: var(--bg-3); color: var(--text); }
  .wheel-row[aria-selected='true'] { background: var(--accent-soft); color: var(--text); }
  .row-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n { color: var(--text-3); font-size: 11px; font-variant-numeric: tabular-nums; }
  .wheel-row.new { color: var(--accent-text); font-weight: 600; }
  .side :global(.editor) { flex: 1; }
</style>
