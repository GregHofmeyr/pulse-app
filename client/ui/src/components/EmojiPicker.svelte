<script lang="ts">
  import { tick } from 'svelte'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Server } from '../lib/protocol/Server'
  import { app } from '../lib/store.svelte'
  import { canonicalKey, groups, mediaUrl, reactionKey, savedTone, saveTone, search, standard, twemojiUrl, withSkin, type Std, type Tone } from '../lib/emoji'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import Icon from './Icon.svelte'

  let { onPick, onClose, servers, memberOf }: {
    /** `keep`: Shift was held, so the caller leaves the picker open for the next pick. */
    onPick: (e: Std | CustomEmoji, opts: { keep: boolean }) => void
    onClose: () => void
    servers: Server[]
    memberOf: Set<string>
  } = $props()

  type Item = Std | CustomEmoji
  type Section = { id: string; label: string; items: Item[]; server?: Server; glyph?: string }

  const COLS = 9
  const MAX_FREQUENT = 27

  let query = $state('')
  let tone = $state<Tone>(savedTone())
  let collapsed = $state<Record<string, boolean>>({})
  let active = $state(0)
  let hovered = $state<Item | null>(null)
  let navigated = $state(false)
  let root: HTMLDivElement
  let searchEl: HTMLInputElement
  let scroller: HTMLDivElement

  function setTone(t: Tone) {
    tone = t
    saveTone(t)
  }

  const isCustom = (e: Item): e is CustomEmoji => 'server_id' in e
  // Keyed on the reaction key: frequent entries can be several tones of one base emoji.
  const keyOf = (e: Item) => reactionKey(e)

  // A frequent key is canonical (no FE0F) and may be a skin-toned variant: index each back to its
  // base emoji and the variant as emoji-data spells it.
  const byKey = new Map<string, { e: Std; u: string }>()
  for (const e of standard) for (const u of [e.u, ...(e.s ?? [])]) byKey.set(canonicalKey(u), { e, u })

  const liveCustom = $derived(
    Object.values(app.state.customEmoji).filter((c) => c.hash !== null && memberOf.has(c.server_id)),
  )

  const sections = $derived.by<Section[]>(() => {
    const out: Section[] = []
    const frequent: Item[] = []
    const seen = new Set<string>()
    for (const raw of app.state.frequentEmoji) {
      if (frequent.length >= MAX_FREQUENT) break
      const k = canonicalKey(raw)
      if (seen.has(k)) continue
      seen.add(k)
      if (k.startsWith('custom:')) {
        const c = app.state.customEmoji[k.slice(7)]
        if (c && c.hash !== null && memberOf.has(c.server_id)) frequent.push(c)
      } else {
        const hit = byKey.get(k)
        // Show the variant that was actually used, not the base.
        if (hit) frequent.push(hit.u === hit.e.u ? hit.e : { ...hit.e, u: hit.u })
      }
    }
    if (frequent.length) out.push({ id: 'frequent', label: 'Frequently used', items: frequent })
    for (const s of servers) {
      if (!memberOf.has(s.id)) continue
      const items = liveCustom.filter((c) => c.server_id === s.id).sort((a, b) => a.name.localeCompare(b.name))
      if (items.length) out.push({ id: `server:${s.id}`, label: s.name, items, server: s })
    }
    groups.forEach((g, i) => {
      const items = standard.filter((e) => e.g === i)
      out.push({ id: `group:${g.key}`, label: g.label, items, glyph: items[0]?.u })
    })
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

  // Collapsing a section shrinks `flat`; keep the highlight on a cell that still exists.
  $effect(() => {
    if (active > flat.length - 1) active = Math.max(0, flat.length - 1)
  })

  $effect(() => {
    searchEl?.focus()
  })

  // A custom emoji starts on its still frame; `animatedEmoji` plays it when the rule allows.
  const imgSrc = (e: Item) => (isCustom(e) ? stillSrc(e) : twemojiUrl(withSkin(e, tone)))
  const nameOf = (e: Item) => (isCustom(e) ? e.name : e.n)
  const picked = (e: Item): Item => (isCustom(e) ? e : { ...e, u: withSkin(e, tone) })

  function pick(e: Item, keep = false) {
    onPick(picked(e), { keep })
    if (keep) searchEl?.focus()
  }

  function jump(id: string) {
    query = '' // the sections are hidden while a search is showing
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
    } else if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft') {
      // In a typed search the caret needs these keys, until ArrowDown enters the grid.
      if (ev.target === searchEl && query && !navigated) return
      ev.preventDefault()
      move(ev.key === 'ArrowRight' ? 1 : -1)
    } else if (ev.key === 'ArrowDown') {
      ev.preventDefault()
      move(COLS)
    } else if (ev.key === 'ArrowUp') {
      ev.preventDefault()
      move(-COLS)
    } else if (ev.key === 'Enter') {
      // The focused cell wins, then the highlighted one; with nothing highlighted only a search's
      // first result is picked, so Enter in a fresh picker never chooses an emoji unseen.
      const el = ev.target as HTMLElement
      const cellEl = el.closest?.('.cell')
      // Any other control (tone, rail, section header) handles its own Enter.
      if (!cellEl && el !== searchEl && el !== root) return
      const target = cellEl
        ? flat.find((e) => keyOf(e) === cellEl.getAttribute('data-key'))
        : navigated || hovered ? flat[active] : results?.[0]
      if (target) {
        ev.preventDefault() // also stops the button's own click from picking twice
        pick(target, ev.shiftKey)
      }
    } else if (ev.key.length === 1 && !ev.ctrlKey && !ev.metaKey && !ev.altKey && ev.target !== searchEl) {
      searchEl?.focus() // the keystroke then lands in the search box
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
    data-key={reactionKey(e)} onclick={(ev) => pick(e, ev.shiftKey)}
    onmousedown={(ev) => { if (ev.shiftKey) ev.preventDefault() /* a Shift pick must not extend a text selection */ }}
    onmouseenter={() => { hovered = e; active = idx }} onmouseleave={() => (hovered = null)}
  >
    <img src={imgSrc(e)} alt="" loading="lazy" draggable="false" {@attach isCustom(e) && animatedEmoji(e)} />
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
          {:else if s.glyph}
            <img class="grp" src={twemojiUrl(s.glyph)} alt="" loading="lazy" />
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
      <!-- The preview of the hovered (or keyboard-highlighted) cell: "on hover" counts the cell's hover. -->
      <img class="big" src={imgSrc(shown)} alt="" {@attach isCustom(shown) && animatedEmoji(shown, () => hovered !== null)} />
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
  .tones { display: flex; gap: 3px; }
  .tone { width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 2px solid transparent; cursor: pointer; }
  .tone.on { border-color: var(--accent); }
  .body { flex: 1; min-height: 0; display: flex; }
  .rail { display: flex; flex-direction: column; gap: 4px; padding: 6px; background: var(--bg-1); border-right: 1px solid var(--line); overflow-y: auto; }
  .rail-btn { width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); cursor: pointer; padding: 0; }
  .rail-btn:hover { background: var(--bg-3); color: var(--text); }
  .grp { width: 20px; height: 20px; }
  .srv { width: 24px; height: 24px; border-radius: 50%; object-fit: cover; }
  .init { display: grid; place-items: center; background: var(--bg-4); color: var(--text); font-size: 12px; font-weight: 700; }
  .grid { flex: 1; min-width: 0; overflow-y: auto; padding: 0 8px 8px; }
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
