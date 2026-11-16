<script lang="ts">
  // A custom time for Send later (#299) or Remind me (#300): a day and a time in your own time (the server keeps it in
  // UTC), checked to be ahead and within a year, then `onPick` with the instant. Placed by its anchor, as the plan form.
  // Enter confirms, Escape or a press outside closes it.
  import { onMount, tick, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { defaultStart, fieldsToIso, isoToFields } from '../lib/plan'
  import { popoverStyle } from '../lib/reactions'
  import { whenLabel, whenReady } from '../lib/schedule'
  import { clock } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'

  let { anchor, title, confirm, initial = null, onPick, onClose }: {
    anchor: HTMLElement
    /** Its heading, e.g. "Send later". */
    title: string
    /** The button's words, e.g. "Schedule". */
    confirm: string
    /** The time it starts at (an instant); none: the next whole hour after an hour from now. */
    initial?: string | null
    /** The time picked (an instant); a rejection's words show and it stays open. */
    onPick: (at: string) => Promise<unknown> | unknown
    onClose: () => void
  } = $props()

  const SIZE = { width: 300, height: 190 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
  const start = untrack(() => (initial ? isoToFields(initial) : defaultStart(Date.now())))
  let date = $state(start.date)
  let time = $state(start.time)
  let busy = $state(false)
  let error = $state('')
  let first: HTMLInputElement | undefined = $state()
  onMount(() => void tick().then(() => first?.focus()))

  const at = $derived(fieldsToIso(date, time))
  const nowMs = $derived(Date.parse(clock.nowIso))
  const ready = $derived(whenReady(at, Date.now()) && !busy)
  const hint = $derived(!at ? 'Pick a day and a time.' : Date.parse(at) <= nowMs ? 'That time has passed.' : whenReady(at, nowMs) ? whenLabel(at, nowMs) : "That's more than a year away.")

  async function go() {
    if (!ready || !at) return
    busy = true
    error = ''
    try {
      await onPick(at)
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      onClose()
    } else if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      void go()
    }
  }
</script>

<div class="when menu-surface pop-in" role="dialog" aria-label={title} tabindex="-1" use:clickOutside={onClose} onkeydown={key} {style}>
  <h3 class="menu-label">{title}</h3>
  <div class="row">
    <label class="field"><span class="menu-label">Day</span><input bind:this={first} class="input input-sm" type="date" bind:value={date} aria-label="Day" /></label>
    <label class="field"><span class="menu-label">Time</span><input class="input input-sm" type="time" bind:value={time} aria-label="Time" /></label>
  </div>
  <small class="hint" aria-live="polite">{hint}</small>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="foot">
    <button type="button" class="btn btn-quiet" onclick={onClose}>Cancel</button>
    <button type="button" class="btn btn-primary" disabled={!ready} onclick={() => void go()}>{confirm}</button>
  </div>
</div>

<style>
  .when { width: 300px; max-width: calc(100vw - 12px); padding: 12px; gap: 8px; color: var(--text); --pop-origin: bottom left; }
  h3 { margin: 0; }
  .row { display: flex; gap: 8px; }
  .field { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .field .menu-label { padding: 0 2px; }
  .hint { color: var(--text-3); font-size: 12px; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .foot { display: flex; justify-content: flex-end; gap: 8px; }
  .foot .btn { height: 30px; padding: 0 12px; font-size: 13px; }
</style>
