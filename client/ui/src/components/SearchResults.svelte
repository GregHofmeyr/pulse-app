<script lang="ts">
  // The results of the last search, in the right-hand column (in place of the member list while open):
  // newest first, each a compact card that shows the message where it lives when clicked.
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Loading from './Loading.svelte'
  import Name from './Name.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { closeSearch, loadMore, openHit, retrySearch, search } from '../lib/search.svelte'
  import { chipText } from '../lib/searchQuery'
  import { lineWithHit, segments } from '../lib/highlight'
  import { relativeTime } from '../lib/conversations'
  import { displayName } from '../lib/selectors'
  import { mediaUrl } from '../lib/emoji'
  import type { SearchHit } from '../lib/protocol/SearchHit'

  const serverId = $derived(search.scope?.kind === 'server' ? search.scope.serverId : null)
  const count = $derived(search.total === null ? '' : search.total > 50 ? '50+ results' : `${search.total} ${search.total === 1 ? 'result' : 'results'}`)
  const nowMs = $derived(Date.parse(clock.nowIso))

  /** What a card shows: a quote's matching line (else its first), any other message's text whole. */
  function shown(h: SearchHit) {
    const kind = app.state.channels[h.message.channel_id]?.kind
    const isQuote = !!h.message.quote || (kind === 'quotes' && !h.message.thread_root_id)
    return isQuote ? lineWithHit(h.text, h.highlights) : { text: h.text, highlights: h.highlights }
  }
  /** An image line's picture when the line a quote card shows is one (its text reads "[image]"):
   *  content has one line per quote line, in order (server `quotes::render_content`). */
  function lineImage(h: SearchHit, s: { text: string; index?: number }) {
    if (s.text !== '[image]' || s.index === undefined) return null
    const media = h.message.quote?.lines[s.index]?.media
    return media ? (media.still_hash ?? media.hash) : null
  }
  function label(h: SearchHit): string | null {
    if (!h.message.thread_root_id) return null
    return app.state.channels[h.message.channel_id]?.kind === 'clips' ? 'reply in clip' : 'reply in quote'
  }
  const author = (h: SearchHit) => (h.message.author_id ? displayName(app.state, serverId, h.message.author_id) : 'Pulse')

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      closeSearch()
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="search-results" aria-label="Search results" onkeydown={keydown}>
  <header>
    <span class="count" aria-live="polite">{search.status === 'loading' ? 'Searching…' : count}</span>
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Close search" data-tip="Close (Esc)" onclick={closeSearch}><Icon name="x" size={16} /></button>
  </header>
  <div class="body">
    {#if search.status === 'error'}
      <div class="state">
        <p role="alert">{search.error || 'Search failed.'}</p>
        <button type="button" class="btn btn-secondary" onclick={() => void retrySearch()}>Retry</button>
      </div>
    {:else if search.status === 'done' && !search.hits.length && !search.nextCursor}
      <div class="state">
        <p>No messages match</p>
        <div class="filters">
          {#if search.words}<span class="pill">{search.words}</span>{/if}
          {#each search.chips as c (chipText(c))}<span class="pill">{chipText(c)}</span>{/each}
        </div>
      </div>
    {:else}
      {#each search.hits as h (h.message.id)}
        {@const s = shown(h)}
        {@const where = app.state.channels[h.message.channel_id]}
        {@const tag = label(h)}
        {@const pic = lineImage(h, s)}
        <button type="button" class="result" onclick={() => openHit(h)}>
          {#if h.message.author_id}<Avatar id={h.message.author_id} name={author(h)} size={28} />{:else}<span class="av-gap"></span>{/if}
          <span class="main">
            <span class="meta">
              {#if h.message.author_id}<Name userId={h.message.author_id} {serverId} text={author(h)} interactive={false} />{:else}<strong>Pulse</strong>{/if}
              {#if serverId && where?.name}<span class="where">#{where.name}</span>{/if}
              <span class="when">{relativeTime(h.message.created_at, nowMs)}</span>
            </span>
            {#if tag}<span class="label">{tag}</span>{/if}
            {#if pic}<img class="line-thumb" src={mediaUrl(pic)} alt="" loading="lazy" decoding="async" />
            {:else}<span class="text" class:one-line={s.text !== h.text}>{#each segments(s.text, s.highlights) as seg, i (i)}{#if seg.hit}<mark>{seg.text}</mark>{:else}{seg.text}{/if}{/each}</span>{/if}
            {#if h.message.clip}<img class="thumb" src={mediaUrl(h.message.clip.thumb_hash)} alt="" loading="lazy" decoding="async" />{/if}
          </span>
        </button>
      {/each}
      {#if search.status === 'loading'}<div class="state"><Loading label="Searching…" /></div>{/if}
      {#if search.error && search.status === 'done'}<p class="state err" role="alert">{search.error}</p>{/if}
      {#if search.nextCursor}
        <button type="button" class="btn btn-quiet more" disabled={search.loadingMore} onclick={() => void loadMore()}>{search.loadingMore ? 'Loading…' : 'Load more'}</button>
      {/if}
    {/if}
  </div>
</section>

<style>
  .search-results { width: var(--right-w, 340px); flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; border-radius: var(--radius); background: var(--bg-1); }
  header { height: 52px; flex-shrink: 0; padding: 0 10px 0 16px; display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid var(--line-soft); }
  .count { font-size: 13px; font-weight: 600; color: var(--text-2); }
  .body { flex: 1; min-height: 0; overflow-y: auto; padding: 8px; display: flex; flex-direction: column; gap: 4px; }
  .result { display: flex; gap: 10px; width: 100%; padding: 8px; border: 0; border-radius: var(--radius-md); background: transparent; color: inherit; font: inherit; text-align: left; }
  .result:hover { background: var(--bg-3); }
  .av-gap { width: 28px; flex-shrink: 0; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .meta { display: flex; align-items: baseline; gap: 6px; font-size: 13px; font-weight: 600; min-width: 0; }
  .where, .when { font-size: 11px; font-weight: 400; color: var(--text-3); white-space: nowrap; }
  .label { font-size: 11px; color: var(--text-3); font-style: italic; }
  .text { font-size: 13px; color: var(--text-body); white-space: pre-wrap; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .text.one-line { -webkit-line-clamp: 1; line-clamp: 1; }
  mark { background: var(--accent-soft); color: var(--text); border-radius: 3px; box-shadow: 0 0 0 1px var(--accent-soft); }
  .thumb { width: 120px; aspect-ratio: 16 / 9; object-fit: cover; border-radius: var(--radius-xs); background: var(--bg-3); }
  /* a quote's image line: the picture whole, small */
  .line-thumb { max-width: 120px; max-height: 90px; object-fit: contain; border-radius: var(--radius-xs); background: var(--bg-3); }
  .state { padding: 24px 12px; display: flex; flex-direction: column; align-items: center; gap: 10px; color: var(--text-3); font-size: 13px; text-align: center; margin: 0; }
  .state p { margin: 0; }
  .state.err { padding: 8px; color: var(--danger-text); }
  .state [role='alert'] { color: var(--danger-text); }
  .filters { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; }
  .pill { padding: 2px 8px; border-radius: var(--radius-xs); background: var(--bg-3); color: var(--text-2); font-size: 12px; }
  .more { align-self: center; margin: 6px 0; }
</style>
