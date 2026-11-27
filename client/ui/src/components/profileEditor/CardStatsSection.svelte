<script lang="ts">
  // The Card stats section's right panel (#727): the lines of the stats panel on your card, each with a switch, in the
  // order they show. Drag a row (railDrag) or press the arrow keys on its handle to reorder; the switched-off ones sit
  // below, in the server's order. Steam completion also carries the perfect games, one line on the card. Everything
  // goes into the editor's draft; nothing saves until Save.
  import { flushSync } from 'svelte'
  import { CARD_STATS, CARD_STAT_LABELS, shownStats, type CardStat } from '../../lib/cardStats'
  import { DRAG_THRESHOLD } from '../../lib/drag'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { railDrag } from '../../lib/railDrag'

  let { state: st }: { state: ProfileEditorState } = $props()

  const on = $derived(shownStats(st.draft.cardStats))
  // the shown ones in their order, then the rest as the server lists them
  const rows = $derived<string[]>([...on, ...CARD_STATS.filter((k) => !on.includes(k))])
  const label = (k: string) => CARD_STAT_LABELS[k as CardStat]

  function move(key: string, to: number) {
    const list = on.filter((k) => k !== key)
    list.splice(Math.max(0, Math.min(to, list.length)), 0, key)
    st.draft.cardStats = list
  }
  // a switch pressed back on joins the end of the shown ones
  function toggle(key: string) {
    st.draft.cardStats = on.includes(key) ? on.filter((k) => k !== key) : [...on, key]
  }

  function handleKeys(e: KeyboardEvent, key: string) {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return
    e.preventDefault()
    if (!on.includes(key)) return
    move(key, on.indexOf(key) + (e.key === 'ArrowUp' ? -1 : 1))
    flushSync()
    document.querySelector<HTMLElement>(`[data-stat="${CSS.escape(key)}"] .handle`)?.focus()
  }

  // A press that travels past the drag threshold is not a click, whether or not a drag began (sideways on this
  // vertical list none does): the click that follows is dropped.
  let press: { x: number; y: number } | null = null
  let travelled = false
  function pressed(e: PointerEvent) {
    press = e.button === 0 ? { x: e.clientX, y: e.clientY } : null
    travelled = false
  }
  function released(e: PointerEvent) {
    if (press) travelled = Math.hypot(e.clientX - press.x, e.clientY - press.y) >= DRAG_THRESHOLD
    press = null
    // the click comes right after the release; if a drag swallowed it, a later one is a fresh click
    if (travelled) setTimeout(() => (travelled = false), 0)
  }
  function flip(key: string) {
    if (travelled) {
      travelled = false
      return
    }
    toggle(key)
  }
</script>

<section class="stats-section">
  <h2>Card stats</h2>
  <ol class="stats" aria-label="Card stats" onpointerdown={pressed} onpointerup={released} onpointercancel={() => (press = null)} use:railDrag={{ item: 'stat', axis: 'y', onDrop: move, disabled: on.length < 2 }}>
    {#each rows as key (key)}
      {@const shown = on.includes(key)}
      <li class="row" class:off={!shown} data-stat={shown ? key : undefined} data-row={key}>
        <button type="button" class="handle" disabled={!shown} aria-label="Move {label(key)}" data-tip="Drag, or use the arrow keys" onkeydown={(e) => handleKeys(e, key)}>
          <svg viewBox="0 0 10 16" width="10" height="16" aria-hidden="true"><circle cx="3" cy="3" r="1.4" /><circle cx="7" cy="3" r="1.4" /><circle cx="3" cy="8" r="1.4" /><circle cx="7" cy="8" r="1.4" /><circle cx="3" cy="13" r="1.4" /><circle cx="7" cy="13" r="1.4" /></svg>
        </button>
        <span class="name" data-tip={key.startsWith('steam') ? 'Shows only while you share Steam' : undefined}>{label(key)}</span>
        <button type="button" class="switch" class:on={shown} role="switch" aria-checked={shown} aria-label={label(key)} data-tip={key.startsWith('steam') ? 'Shows only while you share Steam' : undefined} onclick={() => flip(key)}><span></span></button>
      </li>
    {/each}
  </ol>
  <p class="hint">Steam completion also shows perfect games.</p>
</section>

<style>
  .stats-section { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  h2 { margin: 0; font-size: 15px; font-weight: 600; }
  .stats { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .row { display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 44px; padding: 6px 12px 6px 6px; border-radius: var(--radius-md); background: var(--bg-3); touch-action: none; }
  .row[data-stat] { cursor: grab; }
  .row.off .name { color: var(--text-3); }
  .name { flex: 1 1 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 600; }
  .handle { flex: none; display: grid; place-items: center; width: 24px; height: 32px; padding: 0; border: 0; border-radius: var(--radius-sm); background: none; color: var(--text-3); cursor: grab; }
  .handle:disabled { opacity: .35; cursor: default; }
  .handle svg { fill: currentColor; }
  .handle:hover:not(:disabled) { color: var(--text); }
  .handle:focus-visible, .switch:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .stats :global(.lifted) { position: relative; z-index: 2; box-shadow: var(--shadow-2); transition: none; }
  .stats:global(.dragging) { cursor: grabbing; }
  .stats:global(.settling) :global(*) { transition: none !important; }
  .hint { margin: 0; color: var(--text-3); font-size: 12px; }
</style>
