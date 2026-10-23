<script lang="ts">
  import { tick } from 'svelte'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Server } from '../lib/protocol/Server'
  import { app } from '../lib/store.svelte'
  import { groups, mediaUrl, reactionKey, search, standard, twemojiUrl, withSkin, type Std } from '../lib/emoji'
  import Icon from './Icon.svelte'

  let { onPick, onClose, servers, memberOf }: {
    onPick: (e: Std | CustomEmoji) => void
    onClose: () => void
    servers: Server[]
    memberOf: Set<string>
  } = $props()

  type Item = Std | CustomEmoji
  type Section = { id: string; label: string; items: Item[]; server?: Server }
  type Tone = 0 | 1 | 2 | 3 | 4 | 5

  const COLS = 9
  const MAX_FREQUENT = 27
  const TONE_KEY = 'pulse.skinTone'

  function loadTone(): Tone {
    try {
      const n = Number(localStorage.getItem(TONE_KEY))
      return n >= 1 && n <= 5 ? (n as Tone) : 0
    } catch {
      return 0
    }
  }

  let query = $state('')
  let tone = $state<Tone>(loadTone())
  let collapsed = $state<Record<string, boolean>>({})
  let active = $state(0)
  let hovered = $state<Item | null>(null)
  let navigated = $state(false)
  let root: HTMLDivElement
  let searchEl: HTMLInputElement
  let scroller: HTMLDivElement

  function setTone(t: Tone) {
    tone = t
    try {
      localStorage.setItem(TONE_KEY, String(t))
    } catch {
      /* ignore */
    }
  }

  const isCustom = (e: Item): e is CustomEmoji => 'server_id' in e
  const keyOf = (e: Item) => (isCustom(e) ? `custom:${e.id}` : `${e.g}:${e.n}`)

  // A frequent key may be a skin-toned Unicode variant: index those back to their base emoji.
  const byUnicode = new Map<string, Std>()
  for (const e of standard) {
    byUnicode.set(e.u, e)
    for (const v of e.s ?? []) byUnicode.set(v, e)
  }

  const liveCustom = $derived(
    Object.values(app.state.customEmoji).filter((c) => c.hash !== null && memberOf.has(c.server_id)),
  )

  const sections = $derived.by<Section[]>(() => {
    const out: Section[] = []
    const frequent: Item[] = []
    for (const k of app.state.frequentEmoji) {
      if (frequent.length >= MAX_FREQUENT) break
      if (k.startsWith('custom:')) {
        const c = app.state.customEmoji[k.slice(7)]
        if (c && c.hash !== null && memberOf.has(c.server_id)) frequent.push(c)
      } else {
        const e = byUnicode.get(k)
        // Show the variant that was actually used, not the base.
        if (e) frequent.push(k === e.u ? e : { ...e, u: k })
      }
    }
    if (frequent.length) out.push({ id: 'frequent', label: 'Frequently used', items: frequent })
    for (const s of servers) {
      if (!memberOf.has(s.id)) continue
      const items = liveCustom.filter((c) => c.server_id === s.id).sort((a, b) => a.name.localeCompare(b.name))
      if (items.length) out.push({ id: `server:${s.id}`, label: s.name, items, server: s })
    }
    groups.forEach((g, i) => out.push({ id: `group:${g.key}`, label: g.label, items: standard.filter((e) => e.g === i) }))
    return out
  })

  const results = $derived(query.trim() ? search(query, liveCustom) : null)
  const flat = $derived<Item[]>(results ?? sections.filter((s) => !collapsed[s.id]).flatMap((s) => s.items))
  const shown = $derived(hovered ?? (navigated ? (flat[active] ?? null) : null))

  $effect(() => {
    void query
    active = 0
    navigated = false
  })

  $effect(() => {
    searchEl?.focus()
  })

  const imgSrc = (e: Item, animate = false) =>
    isCustom(e) ? mediaUrl((animate ? e.hash : (e.still_hash ?? e.hash))!) : twemojiUrl(withSkin(e, tone))
  const nameOf = (e: Item) => (isCustom(e) ? e.name : e.n)
  const picked = (e: Item): Item => (isCustom(e) ? e : { ...e, u: withSkin(e, tone) })

  function pick(e: Item) {
    onPick(picked(e))
  }

  function jump(id: string) {
    collapsed[id] = false
    void tick().then(() => root.querySelector(`[data-section="${id}"]`)?.scrollIntoView({ block: 'start' }))
  }

  function move(delta: number) {
    if (!flat.length) return
    navigated = true
    hovered = null
    active = Math.max(0, Math.min(flat.length - 1, active + delta))
    void tick().then(() => (root.querySelector('.cell.active') as HTMLElement | null)?.scrollIntoView?.({ block: 'nearest' }))
  }

  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') {
      ev.preventDefault()
      onClose()
    } else if (ev.key === 'ArrowRight') {
      ev.preventDefault()
      move(1)
    } else if (ev.key === 'ArrowLeft') {
      ev.preventDefault()
      move(-1)
    } else if (ev.key === 'ArrowDown') {
      ev.preventDefault()
      move(COLS)
    } else if (ev.key === 'ArrowUp') {
      ev.preventDefault()
      move(-COLS)
    } else if (ev.key === 'Enter') {
      if (flat[active]) {
        ev.preventDefault()
        pick(flat[active])
      }
    } else if (ev.key.length === 1 && !ev.ctrlKey && !ev.metaKey && !ev.altKey && ev.target !== searchEl) {
      searchEl.focus() // the keystroke then lands in the search box
    }
  }

  const TONES: Tone[] = [0, 1, 2, 3, 4, 5]
  const TONE_COLORS = ['#f7c948', '#f7dbc0', '#e8c29a', '#c89369', '#9a6a45', '#5f402c']
  const startIndex = (s: Section) => {
    let n = 0
    for (const o of sections) {
      if (o === s) break
      if (!collapsed[o.id]) n += o.items.length
    }
    return n
  }
</script>

{#snippet cell(e: Item, idx: number)}
  <button
    class="cell" class:active={navigated && idx === active} type="button" aria-label={`:${nameOf(e)}:`}
    data-key={reactionKey(e)} onclick={() => pick(e)}
    onmouseenter={() => { hovered = e; active = idx }} onmouseleave={() => (hovered = null)}
  >
    <img src={imgSrc(e, isCustom(e) && hovered === e)} alt="" loading="lazy" draggable="false" />
  </button>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="picker" bind:this={root} role="dialog" aria-label="Emoji picker" tabindex="-1" onkeydown={onKey}>
  <div class="top">
    <input bind:this={searchEl} bind:value={query} class="search" type="text" placeholder="Search emoji" aria-label="Search emoji" autocomplete="off" spellcheck="false" />
    <div class="tones" role="group" aria-label="Skin tone">
      {#each TONES as t (t)}
        <button class="tone" class:on={tone === t} type="button" aria-label={t === 0 ? 'Default skin tone' : `Skin tone ${t}`} aria-pressed={tone === t} style:background={TONE_COLORS[t]} onclick={() => setTone(t)}></button>
      {/each}
    </div>
  </div>
  <div class="body">
    <nav class="rail" aria-label="Emoji sections">
      {#each sections as s (s.id)}
        <button class="rail-btn" type="button" title={s.label} aria-label={s.label} data-rail={s.id} onclick={() => jump(s.id)}>
          {#if s.id === 'frequent'}
            <Icon name="clock" size={18} />
          {:else if s.server}
            {#if s.server.icon_hash}
              <img class="srv" src={mediaUrl(s.server.icon_hash)} alt="" />
            {:else}
              <span class="srv init">{s.server.name.trim().charAt(0).toUpperCase()}</span>
            {/if}
          {:else if 'u' in s.items[0]}
            <img class="grp" src={twemojiUrl(s.items[0].u)} alt="" loading="lazy" />
          {/if}
        </button>
      {/each}
    </nav>
    <div class="grid" bind:this={scroller}>
      {#if results}
        {#if results.length === 0}
          <p class="none">No emoji found</p>
        {:else}
          <div class="cells">
            {#each results as e, i (keyOf(e))}{@render cell(e, i)}{/each}
          </div>
        {/if}
      {:else}
        {#each sections as s (s.id)}
          <section data-section={s.id}>
            <button class="head" type="button" aria-expanded={!collapsed[s.id]} onclick={() => (collapsed[s.id] = !collapsed[s.id])}>
              <span class="caret" class:shut={collapsed[s.id]}><Icon name="chevron" size={12} /></span>{s.label}
            </button>
            {#if !collapsed[s.id]}
              {@const base = startIndex(s)}
              <div class="cells">
                {#each s.items as e, i (keyOf(e))}{@render cell(e, base + i)}{/each}
              </div>
            {/if}
          </section>
        {/each}
      {/if}
    </div>
  </div>
  <div class="foot">
    {#if shown}
      <img class="big" src={imgSrc(shown, true)} alt="" />
      <span class="nm">:{nameOf(shown)}:</span>
    {:else}
      <span class="credit">Emoji graphics: Twemoji, CC-BY 4.0</span>
    {/if}
  </div>
</div>

<style>
  .picker { width: 360px; height: 420px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45); overflow: hidden; color: var(--text); outline: none; }
  .top { display: flex; gap: 8px; align-items: center; padding: 10px; border-bottom: 1px solid var(--line); }
  .search { flex: 1; min-width: 0; background: var(--bg-0); color: var(--text); border: 1px solid var(--line); border-radius: 8px; padding: 6px 10px; font: inherit; }
  .search:focus { outline: none; border-color: var(--accent); }
  .tones { display: flex; gap: 3px; }
  .tone { width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 2px solid transparent; cursor: pointer; }
  .tone.on { border-color: var(--accent); }
  .body { flex: 1; min-height: 0; display: flex; }
  .rail { display: flex; flex-direction: column; gap: 4px; padding: 6px; background: var(--bg-1); border-right: 1px solid var(--line); overflow-y: auto; scrollbar-width: thin; scrollbar-color: var(--bg-4) transparent; }
  .rail-btn { width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); cursor: pointer; padding: 0; }
  .rail-btn:hover { background: var(--bg-3); color: var(--text); }
  .grp { width: 20px; height: 20px; }
  .srv { width: 24px; height: 24px; border-radius: 50%; object-fit: cover; }
  .init { display: grid; place-items: center; background: var(--bg-4); color: var(--text); font-size: 12px; font-weight: 700; }
  .grid { flex: 1; min-width: 0; overflow-y: auto; padding: 0 8px 8px; scrollbar-width: thin; scrollbar-color: var(--bg-4) transparent; }
  .grid::-webkit-scrollbar, .rail::-webkit-scrollbar { width: 8px; }
  .grid::-webkit-scrollbar-thumb, .rail::-webkit-scrollbar-thumb { background: var(--bg-4); border-radius: 4px; }
  .grid::-webkit-scrollbar-track, .rail::-webkit-scrollbar-track { background: transparent; }
  .head { position: sticky; top: 0; z-index: 1; width: 100%; display: flex; align-items: center; gap: 4px; padding: 8px 2px 4px; background: var(--bg-2); border: 0; color: var(--text-3); font: inherit; font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; cursor: pointer; text-align: left; }
  .caret { display: inline-flex; transition: transform 0.12s; }
  .caret.shut { transform: rotate(-90deg); }
  .cells { display: grid; grid-template-columns: repeat(9, 1fr); gap: 2px; }
  .cell { aspect-ratio: 1; display: grid; place-items: center; border: 0; border-radius: 6px; background: transparent; padding: 0; cursor: pointer; }
  .cell img { width: 24px; height: 24px; object-fit: contain; }
  .cell:hover, .cell.active { background: var(--bg-4); }
  .none { color: var(--text-3); text-align: center; margin: 24px 0; }
  .foot { height: 44px; display: flex; align-items: center; gap: 10px; padding: 0 12px; border-top: 1px solid var(--line); background: var(--bg-1); }
  .big { width: 28px; height: 28px; object-fit: contain; }
  .nm { color: var(--text-2); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .credit { color: var(--text-3); font-size: 12px; }
</style>
