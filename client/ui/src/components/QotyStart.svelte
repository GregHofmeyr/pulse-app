<script lang="ts">
  // Start a Quote of the Year vote (year and length), or reopen one that was ended early (same
  // lengths, no year). The server checks everything; this only asks.
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import { modal } from '../lib/actions'
  import { defaultQotyYear } from '../lib/quotes'
  import { api, errorText } from '../lib/tauri'

  let { serverId, reopen = null, onClose }: {
    serverId: string
    /** The event to reopen; absent to start a new one. */
    reopen?: string | null
    onClose: () => void
  } = $props()

  type Length = 'hours' | 'day' | 'custom' | 'open'
  type Unit = 'minutes' | 'hours'
  const LENGTHS: { value: Length; label: string }[] = [
    { value: 'hours', label: '3 hours' },
    { value: 'day', label: '1 day' },
    { value: 'custom', label: 'Custom' },
    { value: 'open', label: 'Until ended' },
  ]
  const UNITS: { value: Unit; label: string }[] = [
    { value: 'minutes', label: 'minutes' },
    { value: 'hours', label: 'hours' },
  ]
  /** Custom lengths: 5 minutes to 14 days (spec §10). */
  const MIN_MINUTES = 5
  const MAX_MINUTES = 14 * 24 * 60

  let year = $state(defaultQotyYear(new Date()))
  let length = $state<Length>('hours')
  let amount = $state(30)
  let unit = $state<Unit>('minutes')
  let busy = $state(false)
  let error = $state('')

  /** The length in minutes (null = until ended), or undefined while a custom one is out of range. */
  const minutes = $derived.by((): number | null | undefined => {
    if (length === 'hours') return 180
    if (length === 'day') return 1440
    if (length === 'open') return null
    const m = amount * (unit === 'hours' ? 60 : 1)
    return Number.isInteger(m) && m >= MIN_MINUTES && m <= MAX_MINUTES ? m : undefined
  })
  const yearOk = $derived(Number.isInteger(year) && year >= 1970 && year <= 9999)
  const ready = $derived(minutes !== undefined && (reopen !== null || yearOk))
  const title = $derived(reopen ? 'Reopen voting' : 'Start Quote of the Year')

  async function go() {
    if (!ready || busy) return
    busy = true
    error = ''
    const m = minutes ?? null
    try {
      if (reopen) await api.reopenQoty(reopen, m ?? undefined)
      else await api.startQoty(serverId, { year, minutes: m })
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const cancel = () => {
    if (!busy) onClose()
  }
</script>

<div class="overlay">
  <div class="backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <!-- not a <form>: the length buttons (Segmented) would submit it -->
  <div class="modal" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <div class="top">
      <span class="qoty-mark mark" aria-hidden="true">“</span>
      <h1>{title}</h1>
      <button type="button" class="close" aria-label="Cancel" disabled={busy} onclick={cancel}><Icon name="x" /></button>
    </div>
    {#if !reopen}
      <label class="field">
        <span class="lbl">Year</span>
        <input type="number" aria-label="Year" min="1970" max="9999" bind:value={year} disabled={busy} />
      </label>
    {:else}
      <p class="hint">Votes and added quotes are kept. Pick how long it runs this time.</p>
    {/if}
    <div class="field">
      <span class="lbl">How long</span>
      <Segmented label="Length" options={LENGTHS} value={length} onchange={(v) => (length = v)} />
    </div>
    {#if length === 'custom'}
      <div class="custom">
        <input type="number" aria-label="Custom length" min="1" bind:value={amount} disabled={busy} />
        <Segmented label="Unit" options={UNITS} value={unit} onchange={(v) => (unit = v)} />
      </div>
      <p class="hint" class:bad={minutes === undefined}>Anything from 5 minutes to 14 days.</p>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="qoty-btn go" disabled={!ready || busy} onclick={go}>{reopen ? 'Reopen' : 'Start'}</button>
    </div>
  </div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: var(--scrim); }
  .modal { position: relative; width: min(440px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; padding: 22px; border-radius: 16px; background: var(--bg-1); border: 1px solid var(--line); display: flex; flex-direction: column; gap: 14px; outline: none; }
  .top { display: flex; align-items: center; gap: 10px; }
  .mark { font-size: 34px; padding-top: 12px; color: var(--quote-green); }
  h1 { flex: 1; margin: 0; font-size: 18px; font-weight: 600; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  .close:hover { background: var(--bg-4); color: var(--text); }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .lbl { font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  input { height: 38px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-2); }
  .field input { width: 120px; }
  .custom { display: flex; align-items: center; gap: 10px; }
  .custom input { width: 96px; }
  .hint { margin: 0; font-size: 12px; color: var(--text-3); }
  .hint.bad { color: var(--warn); }
  .err { margin: 0; font-size: 13px; color: var(--danger); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .ghost { height: 36px; padding: 0 14px; border-radius: 10px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); }
  .ghost:hover:not(:disabled) { background: var(--bg-3); color: var(--text); }
  .go { height: 38px; padding: 0 18px; border-radius: 10px; font-size: 14px; }
  .ghost:disabled { opacity: .6; }
</style>
