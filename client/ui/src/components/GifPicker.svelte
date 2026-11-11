<script lang="ts" module>
  /** Searching waits this long after the last keystroke (GIPHY's beta keys allow 100 calls an hour per server). */
  export const SEARCH_DELAY_MS = 350
</script>

<script lang="ts">
  // The GIF picker (#58), in the emoji picker's frame: a search box, a rail (GIPHY: trending, or what you search;
  // Favourites), a two-column grid and "Powered by GIPHY" in the footer (GIPHY's terms). The core searches GIPHY
  // itself with the server's key; results come in GIPHY's order with GIPHY's URLs untouched, and nothing is cached.
  // Picking one sends it; the star keeps it in your favourites.
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { animatedGif } from '../lib/animate.svelte'
  import { gifLink, gridMedia, masonry } from '../lib/giphy'
  import { gifFavs, isGifFav, loadGifFavs, toggleGifFav } from '../lib/gifs.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText, type GifResult } from '../lib/tauri'
  import type { Gif } from '../lib/protocol/Gif'

  let { onPick, onClose }: {
    /** `link`: what the message says (the GIF's GIPHY page). */
    onPick: (gif: Gif, link: string) => void
    onClose: () => void
  } = $props()

  const NEAR_END_PX = 300

  type Tab = 'giphy' | 'favourites'
  /** One grid cell: a GIPHY result, or a favourite drawn from GIPHY's media links for its id. */
  type Cell = { gif: Gif; link: string; anim: string; still: string; w: number; h: number }

  let tab = $state<Tab>('giphy')
  let query = $state('')
  let results = $state<GifResult[]>([])
  /** The search `results` are for ('' = trending) and the next page's offset. */
  let shownQuery = ''
  let next = $state<number | null>(null)
  let loading = $state(false)
  let error = $state('')
  let notSetUp = $state(false)
  let hovered = $state<string | null>(null)
  let searchEl: HTMLInputElement | undefined = $state()
  let scroller: HTMLDivElement | undefined = $state()
  // a newer search makes an older answer moot
  let asked = 0

  const fromResult = (r: GifResult): Cell => ({
    gif: { id: r.id, title: r.title, width: r.width, height: r.height }, link: gifLink(r),
    anim: r.preview, still: r.still, w: r.preview_width, h: r.preview_height,
  })
  const fromFav = (g: Gif): Cell => ({ gif: g, link: `https://giphy.com/gifs/${g.id}`, ...gridMedia(g.id), w: g.width, h: g.height })

  const cells = $derived(tab === 'favourites' ? gifFavs.list.map(fromFav) : results.map(fromResult))
  const columns = $derived(masonry(cells, (c) => c.h / c.w))

  async function load(q: string, offset: number) {
    const mine = ++asked
    loading = true
    error = ''
    try {
      const page = await api.gifFeed(q, offset)
      if (mine !== asked) return
      if (!page) {
        notSetUp = true
        results = []
        next = null
        return
      }
      shownQuery = q
      // a GIF already shown (GIPHY's pages can overlap) isn't shown twice
      const have = new Set(offset ? results.map((r) => r.id) : [])
      const fresh = page.gifs.filter((g) => !have.has(g.id) && !!have.add(g.id))
      results = offset ? [...results, ...fresh] : fresh
      next = page.next
      if (!offset) scroller?.scrollTo?.({ top: 0 })
    } catch (e) {
      if (mine === asked) error = errorText(e)
    } finally {
      if (mine === asked) loading = false
    }
  }

  // trending at once on opening, then each search a moment after typing stops
  let first = true
  $effect(() => {
    const q = query.trim()
    return untrack(() => {
      if (q && tab === 'favourites') tab = 'giphy'
      if (first) {
        first = false
        void load(q, 0)
        return
      }
      const t = setTimeout(() => void load(q, 0), SEARCH_DELAY_MS)
      return () => clearTimeout(t)
    })
  })

  $effect(() => {
    const me = app.state.me?.id
    if (me) untrack(() => void loadGifFavs(me, true))
  })

  $effect(() => {
    searchEl?.focus()
  })

  function more() {
    // after a failed page, only Try again asks again: scrolling mustn't spend the server's GIPHY calls retrying
    if (tab !== 'giphy' || loading || error || next === null || !scroller) return
    if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - NEAR_END_PX) void load(shownQuery, next)
  }

  function star(g: Gif) {
    toggleGifFav(g).catch((e) => (error = errorText(e)))
  }

  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') {
      ev.preventDefault()
      onClose()
    } else if (ev.key === 'Enter' && ev.target === searchEl) {
      // Enter in the box sends the first GIF showing, but only once it's a result of what's typed: never the
      // previous results while the search is still on its way
      ev.preventDefault()
      const current = tab === 'favourites' || (!loading && query.trim() === shownQuery)
      if (current && cells[0]) onPick(cells[0].gif, cells[0].link)
    }
  }
</script>

{#snippet cell(c: Cell)}
  {@const starred = isGifFav(c.gif.id)}
  <div class="cell" style:aspect-ratio="{c.w} / {c.h}">
    <button class="pick" type="button" aria-label={c.gif.title || 'GIF'} onclick={() => onPick(c.gif, c.link)}
      onmouseenter={() => (hovered = c.gif.title)} onmouseleave={() => (hovered = null)} onfocus={() => (hovered = c.gif.title)} onblur={() => (hovered = null)}>
      <img src={c.still} alt="" loading="lazy" draggable="false" {@attach animatedGif(c.anim, c.still)} />
    </button>
    <button class="icon-btn icon-btn-xs icon-btn-overlay star" class:on={starred} type="button" aria-pressed={starred}
      aria-label={starred ? 'Remove from favourite GIFs' : 'Add to favourite GIFs'} onclick={() => star(c.gif)}><Icon name="star" size={14} /></button>
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="picker" role="dialog" aria-label="GIF picker" tabindex="-1" onkeydown={onKey}>
  <div class="top">
    <input bind:this={searchEl} bind:value={query} class="search input input-sm" type="text" placeholder="Search GIPHY" aria-label="Search GIPHY"
      autocomplete="off" spellcheck="false" maxlength="50" disabled={notSetUp} />
  </div>
  <div class="body">
    <nav class="rail" aria-label="GIF sections">
      <button class="rail-btn" class:on={tab === 'giphy'} type="button" aria-label={query.trim() ? 'Search results' : 'Trending'} aria-pressed={tab === 'giphy'}
        data-tip={query.trim() ? 'Search results' : 'Trending'} onclick={() => (tab = 'giphy')}><Icon name="trending" size={18} /></button>
      <button class="rail-btn" class:on={tab === 'favourites'} type="button" aria-label="Favourites" aria-pressed={tab === 'favourites'}
        data-tip="Favourites" onclick={() => (tab = 'favourites')}><Icon name="star" size={18} /></button>
    </nav>
    <div class="grid" bind:this={scroller} onscroll={more}>
      {#if notSetUp}
        <p class="none">GIFs aren't set up on this server yet</p>
      {:else if error && !cells.length}
        <p class="none" role="alert">{error}</p>
      {:else if tab === 'favourites' && !cells.length}
        <p class="none">{gifFavs.loaded ? 'Star a GIF to keep it here' : 'Loading…'}</p>
      {:else if !cells.length}
        <p class="none">{loading ? 'Loading…' : query.trim() ? 'No GIFs found' : 'Nothing trending right now'}</p>
      {:else}
        <div class="cols">
          {#each columns as col, i (i)}
            <div class="col">{#each col as c (c.gif.id)}{@render cell(c)}{/each}</div>
          {/each}
        </div>
        {#if error}<p class="none" role="alert">{error}{#if next !== null} <button type="button" class="link-btn" onclick={() => void load(shownQuery, next!)}>Try again</button>{/if}</p>{/if}
      {/if}
    </div>
  </div>
  <div class="foot">
    <span class="nm">{hovered ?? ''}</span>
    <span class="credit">Powered by GIPHY</span>
  </div>
</div>

<style>
  /* the emoji picker's frame, rail, grid spacing and footer (EmojiPicker.svelte) */
  .picker { width: 360px; height: 420px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); overflow: hidden; color: var(--text); outline: none; }
  .top { display: flex; gap: 8px; align-items: center; padding: 10px; border-bottom: 1px solid var(--line); }
  .search { flex: 1; min-width: 0; }
  .body { flex: 1; min-height: 0; display: flex; }
  .rail { display: flex; flex-direction: column; gap: 4px; padding: 6px; background: var(--bg-1); border-right: 1px solid var(--line); overflow-y: auto; }
  .rail-btn { width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); cursor: pointer; padding: 0; }
  .rail-btn:hover, .rail-btn.on { background: var(--bg-3); color: var(--text); }
  .grid { flex: 1; min-width: 0; overflow-y: auto; padding: 8px; }
  .cols { display: flex; gap: 4px; align-items: flex-start; }
  .col { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .cell { position: relative; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-3); }
  .pick { display: block; width: 100%; height: 100%; padding: 0; border: 0; background: none; cursor: pointer; }
  .pick img { display: block; width: 100%; height: 100%; object-fit: cover; }
  .pick:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  .star { position: absolute; top: 4px; right: 4px; opacity: 0; }
  .cell:hover .star, .star:focus-visible { opacity: 1; }
  .star.on :global(svg) { fill: currentColor; }
  .none { color: var(--text-3); font-size: 13px; text-align: center; margin: 24px 0; }
  .foot { height: 44px; display: flex; align-items: center; gap: 10px; padding: 0 12px; border-top: 1px solid var(--line); background: var(--bg-1); }
  .nm { flex: 1; min-width: 0; color: var(--text-2); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .credit { flex-shrink: 0; color: var(--text-3); font-size: 12px; font-weight: 700; }
</style>
