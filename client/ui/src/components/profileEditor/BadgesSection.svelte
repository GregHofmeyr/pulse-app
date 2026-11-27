<script lang="ts">
  // The Badges section's right panel (#727): the five slots of your card in a row, and a grid of every badge you hold.
  // The slots are the card's showcase: drag to reorder (railDrag), drag one out of the row to take it off, or press
  // the arrow keys on a focused slot. A grid tile adds or removes with a click, or drags into the row. "Use my rarest
  // five" is the `auto` showcase. Everything goes into the editor's state; nothing saves until Save.
  import { flushSync, untrack } from 'svelte'
  import BadgeIcon from '../BadgeIcon.svelte'
  import { SHOWCASE_MAX, badgeStore, fallbackName } from '../../lib/badges.svelte'
  import { DRAG_THRESHOLD } from '../../lib/drag'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { railDrag } from '../../lib/railDrag'
  import type { Badge } from '../../lib/protocol/Badge'

  let { state: st }: { state: ProfileEditorState } = $props()

  const held = $derived(new Map<string, Badge>(st.profile.badges.map((b) => [b.key, b])))
  const shown = $derived(st.shown)
  const auto = $derived(st.showcase === 'auto')
  // the five places; the empty ones draw a dashed target
  const places = $derived(Array.from({ length: SHOWCASE_MAX }, (_, i) => shown[i] ?? null))
  $effect(() => {
    if (st.profile.badges.length) untrack(() => void badgeStore.load())
  })

  const nameOf = (key: string) => {
    const d = badgeStore.def(key)
    return d?.name ?? (held.get(key)?.hidden ? '???' : fallbackName(key))
  }
  /** How many people have the tier you hold, when the catalogue says; nothing is shown without it. */
  function holders(key: string): string {
    const b = held.get(key)
    const d = badgeStore.def(key)
    const n = d?.tiers[(b?.tier ?? 1) - 1]?.holders
    if (!b || b.hidden || !d || n === undefined) return ''
    if (d.one_of_a_kind) return '1 of 1'
    return `${n} ${n === 1 ? 'has' : 'have'} this`
  }

  function own(): string[] {
    // the first edit away from the rarest five starts from them
    if (st.showcase === 'auto') st.showcase = st.shown
    return st.showcase as string[]
  }
  function move(key: string, to: number) {
    const list = own().filter((k) => k !== key)
    list.splice(Math.max(0, Math.min(to, list.length)), 0, key)
    st.showcase = list
  }
  function toggle(key: string) {
    const list = own()
    if (list.includes(key)) st.showcase = list.filter((k) => k !== key)
    else if (list.length < SHOWCASE_MAX) st.showcase = [...list, key]
  }
  const toggleAuto = () => (st.showcase = auto ? [...st.shown] : 'auto')

  function slotKeys(e: KeyboardEvent, key: string) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
    e.preventDefault()
    move(key, shown.indexOf(key) + (e.key === 'ArrowLeft' ? -1 : 1))
    flushSync()
    document.querySelector<HTMLElement>(`[data-badge="${CSS.escape(key)}"]`)?.focus()
  }

  // --- drag out of the row, and from the grid into it (the row's own reordering is railDrag's) ---
  let row = $state<HTMLElement>()
  let slotDown: { key: string; x: number; y: number } | null = null
  let tileDown: { key: string; x: number; y: number } | null = null
  let dragging = $state<string | null>(null)
  let over = $state(false)

  const far = (d: { x: number; y: number }, e: PointerEvent) => Math.hypot(e.clientX - d.x, e.clientY - d.y) >= DRAG_THRESHOLD
  function inside(e: PointerEvent): boolean {
    const r = row?.getBoundingClientRect()
    return !!r && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom
  }
  /** Where `key` would land among the filled slots at pointer `x`: the count of the others whose centre is left of it. */
  function indexAt(x: number, key: string): number {
    const mids = [...(row?.querySelectorAll<HTMLElement>('[data-badge]') ?? [])]
      .filter((el) => el.dataset.badge !== key)
      .map((el) => {
        const r = el.getBoundingClientRect()
        return r.left + r.width / 2
      })
    return mids.filter((m) => m < x).length
  }
  const swallow = (e: MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
  }

  function slotPress(e: PointerEvent) {
    const el = (e.target as Element).closest?.<HTMLElement>('[data-badge]')
    slotDown = e.button === 0 && el ? { key: el.dataset.badge!, x: e.clientX, y: e.clientY } : null
  }
  function tilePress(e: PointerEvent) {
    const el = (e.target as Element).closest?.<HTMLElement>('[data-tile]')
    tileDown = e.button === 0 && el && !(el as HTMLButtonElement).disabled ? { key: el.dataset.tile!, x: e.clientX, y: e.clientY } : null
  }
  function moved(e: PointerEvent) {
    if (!tileDown) return
    if (!dragging && far(tileDown, e)) dragging = tileDown.key
    if (dragging) over = inside(e)
  }
  function released(e: PointerEvent) {
    const s = slotDown
    const t = tileDown
    slotDown = tileDown = null
    const key = dragging
    dragging = null
    over = false
    // a slot pulled out of the row comes off the card (railDrag has already settled any reorder)
    if (s && far(s, e) && !inside(e) && held.has(s.key) && shown.includes(s.key)) {
      own()
      st.showcase = (st.showcase as string[]).filter((k) => k !== s.key)
    }
    if (!t || !key) return
    // the click this drag ends with is not a tile click
    window.addEventListener('click', swallow, { capture: true, once: true })
    setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0)
    if (!inside(e)) return
    const list = own()
    if (!list.includes(key) && list.length >= SHOWCASE_MAX) return
    const to = indexAt(e.clientX, key)
    if (list.includes(key)) move(key, to)
    else st.showcase = [...list.slice(0, to), key, ...list.slice(to)]
  }
</script>

<svelte:window onpointermove={moved} onpointerup={released} onpointercancel={() => (slotDown = tileDown = dragging = null)} />

<div class="badges">
  {#if held.size === 0}
    <p class="muted">No badges yet. The ones you earn can go on your card.</p>
  {:else}
    <section>
      <div class="head">
        <h3>On your card</h3>
        <button type="button" class="chip" class:on={auto} aria-pressed={auto} data-tip="Your five rarest, following new ones as you earn them" onclick={toggleAuto}>Use my rarest five</button>
      </div>
      <ol class="slots" class:over aria-label="Badges on your card" bind:this={row} onpointerdown={slotPress} use:railDrag={{ item: 'badge', onDrop: move, disabled: shown.length < 2 }}>
        {#each places as key, i (key ?? `empty-${i}`)}
          {@const b = key ? held.get(key) : undefined}
          {#if key && b}
            {@const d = badgeStore.def(key)}
            <li>
              <button type="button" class="slot" data-badge={key} aria-label="{nameOf(key)}: take off your card" data-tip="{nameOf(key)} · drag to move, click to take off" onkeydown={(e) => slotKeys(e, key)} onclick={() => toggle(key)}>
                <BadgeIcon icon={d?.icon ?? '🏆'} art={d?.art} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="case" label={nameOf(key)} />
              </button>
            </li>
          {:else}
            <li class="empty" aria-hidden="true"></li>
          {/if}
        {/each}
      </ol>
      <p class="hint">Drag to reorder, or use the arrow keys.</p>
    </section>
    <section>
      <h3>Your badges</h3>
      <div class="grid" role="group" aria-label="Your badges" onpointerdown={tilePress}>
        {#each st.profile.badges as b (b.key)}
          {@const d = badgeStore.def(b.key)}
          {@const on = shown.includes(b.key)}
          <button type="button" class="tile" class:on class:lifted={dragging === b.key} data-tile={b.key} aria-pressed={on} aria-label={nameOf(b.key)} disabled={!on && shown.length >= SHOWCASE_MAX} onclick={() => toggle(b.key)}>
            <BadgeIcon icon={d?.icon ?? '🏆'} art={d?.art} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="case" />
            <span class="name">{nameOf(b.key)}</span>
            {#if holders(b.key)}<span class="count">{holders(b.key)}</span>{/if}
          </button>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .badges { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
  section { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px; min-width: 0; }
  h3 { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .muted, .hint { margin: 0; color: var(--text-3); font-size: 12px; }
  .chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 4px 12px; border: 1px solid var(--line-strong); border-radius: 999px; background: var(--bg-3); color: var(--text-2); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .chip:hover { background: var(--bg-4); color: var(--text); }
  .chip.on { border-color: var(--focus-ring); background: var(--accent-soft); color: var(--accent-text); }
  .chip:focus-visible, .slot:focus-visible, .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  /* five slots of up to 84px in a soft band; they shrink together with the window, and never spill */
  .slots { list-style: none; margin: 0; padding: 12px; display: grid; grid-template-columns: repeat(5, minmax(0, 84px)); gap: 8px; justify-content: start; box-sizing: border-box; max-width: 100%; border-radius: var(--radius-lg); background: var(--accent-faint); border: 1px solid transparent; }
  .slots.over { border-color: var(--focus-ring); }
  .slots > li { aspect-ratio: 1; min-width: 0; }
  .slots > li.empty { border: 2px dashed var(--line-strong); border-radius: var(--radius-md); }
  .slot { display: grid; place-items: center; width: 100%; height: 100%; padding: 0; border: 0; border-radius: var(--radius-md); background: var(--bg-3); cursor: grab; touch-action: none; }
  .slot:hover { background: var(--bg-4); }
  .slots :global(.lifted) { position: relative; z-index: 2; box-shadow: var(--shadow-2); transition: none; }
  .slots:global(.dragging) { cursor: grabbing; }
  .slots:global(.settling) :global(*) { transition: none !important; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 10px; min-width: 0; }
  .tile { display: flex; flex-direction: column; align-items: center; gap: 6px; min-width: 0; padding: 14px 8px 10px; border: 2px solid transparent; border-radius: var(--radius-lg); background: var(--bg-3); color: var(--text); font: inherit; cursor: pointer; touch-action: none; }
  .tile:hover:not(:disabled) { background: var(--bg-4); }
  .tile.on { border-color: var(--focus-ring); }
  .tile.lifted { opacity: .55; }
  .tile:disabled { opacity: .5; cursor: not-allowed; }
  .name, .count { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name { font-size: 13px; font-weight: 600; }
  .count { font-size: 11px; color: var(--text-3); }
</style>
