<script lang="ts">
  // One build as it reads (#618): the caption `BUILDS · <GAME>` above its title, "who · saved · updated", its video and
  // up to four screenshot tiles ("+N" on the fourth when there are more), then its notes as message markdown. Props
  // only, no store: the build page, the side panel (#619) and the pop-out window (#620) each feed it.
  import type { Snippet } from 'svelte'
  import { renderMarkdown } from '../lib/markdown'
  import type { BuildScreenshot as Screenshot } from '../lib/builds'
  import type { BuildLink } from '../lib/protocol/BuildLink'

  let { title, gameName, guessed = false, authorName, savedAt, updatedAt, notes, screenshots, link, archived, quickDrop = false, onImage, video, actions }: {
    title: string
    gameName: string | null
    /** The game was guessed from the link's title. */
    guessed?: boolean
    authorName: string
    /** When it was saved and last updated, as shown ("2h ago"). */
    savedAt: string
    updatedAt: string | null
    notes: string | null
    screenshots: Screenshot[]
    link: BuildLink | null
    archived: boolean
    quickDrop?: boolean
    /** A screenshot was clicked (its index); without it the tiles are only shown. */
    onImage?: (i: number) => void
    /** The media area's video (the page's player); none draws no video. */
    video?: Snippet
    /** Drawn at the header's right (the page's Actions menu). */
    actions?: Snippet
  } = $props()

  const MAX_TILES = 4
  const tiles = $derived(screenshots.slice(0, MAX_TILES))
  const more = $derived(screenshots.length - MAX_TILES)
  const caption = $derived(gameName ? `Builds · ${gameName}` : 'Builds')
  const meta = $derived([authorName, `saved ${savedAt}`, ...(updatedAt ? [`updated ${updatedAt}`] : [])].join(' · '))
  const html = $derived(notes?.trim() ? renderMarkdown(notes) : '')
</script>

<article class="build-view" class:archived>
  <header>
    <div class="heading">
      <span class="caption">{caption}{#if gameName && guessed}<span class="guessed"> · guessed</span>{/if}</span>
      <h1>{title}</h1>
      <span class="meta">{meta}</span>
      {#if archived || quickDrop}
        <span class="badges">
          {#if quickDrop}<span class="badge">QUICK DROP</span>{/if}
          {#if archived}<span class="badge">ARCHIVED</span>{/if}
        </span>
      {/if}
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  {#if (video && link) || tiles.length}
    <div class="media">
      {#if video && link}<div class="video">{@render video()}</div>{/if}
      {#if tiles.length}
        <div class="tiles" class:one={tiles.length === 1}>
          {#each tiles as s, i (s.id)}
            <svelte:element this={onImage ? 'button' : 'div'} type={onImage ? 'button' : undefined} class="tile" role={onImage ? undefined : 'presentation'}
              aria-label={onImage ? `Open screenshot ${i + 1}${i === MAX_TILES - 1 && more > 0 ? `, and ${more} more` : ''}` : undefined} onclick={onImage ? () => onImage(i) : undefined}
              style:aspect-ratio={s.w && s.h ? `${s.w} / ${s.h}` : undefined}>
              {#if s.url}<img src={s.url} alt="" loading="lazy" decoding="async" draggable="false" />{/if}
              {#if i === MAX_TILES - 1 && more > 0}<span class="more">+{more}</span>{/if}
            </svelte:element>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
  {#if html}
    <!-- the message markdown, rendered and sanitised as a message's is -->
    <div class="notes">{@html html}</div>
  {/if}
</article>

<style>
  .build-view { display: flex; flex-direction: column; gap: 14px; min-width: 0; }
  .build-view.archived .media, .build-view.archived .notes { opacity: .7; }
  header { display: flex; align-items: flex-start; gap: 12px; }
  .heading { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .caption { font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); overflow-wrap: anywhere; }
  .guessed { text-transform: none; letter-spacing: 0; font-weight: 400; font-style: italic; }
  h1 { margin: 0; font-size: 22px; font-weight: 700; color: var(--text); overflow-wrap: anywhere; }
  .meta { font-size: 13px; color: var(--text-3); overflow-wrap: anywhere; }
  .badges { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .badge { padding: 1px 7px; border-radius: var(--radius-xs); border: 1px solid var(--bg-4); color: var(--text-2); font-size: 10px; font-weight: 700; letter-spacing: .04em; }
  .actions { flex-shrink: 0; }
  .media { display: flex; flex-direction: column; gap: 8px; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(140px, 100%), 1fr)); gap: 8px; }
  .tiles.one { grid-template-columns: minmax(0, 480px); }
  .tile { position: relative; display: block; padding: 0; border: 0; border-radius: var(--radius-sm); overflow: hidden; background: var(--bg-3); aspect-ratio: 16 / 9; max-height: 320px; }
  button.tile { cursor: zoom-in; }
  .tile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .more { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: var(--scrim-strong); color: var(--tooltip-text); font-size: 20px; font-weight: 700; }
  .notes { font-size: 14px; line-height: 1.5; color: var(--text); overflow-wrap: anywhere; }
  .notes :global(p) { margin: 0 0 8px; }
  .notes :global(ul), .notes :global(ol) { margin: 0 0 8px; padding-left: 22px; }
  /* nothing in the notes runs out of a narrow column (the pinned build, #638) */
  .notes :global(pre) { white-space: pre-wrap; overflow-wrap: anywhere; }
  .notes :global(img) { max-width: 100%; height: auto; }
</style>
