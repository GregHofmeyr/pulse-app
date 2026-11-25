<script lang="ts">
  // One build as it reads (#618): the caption `BUILDS · <GAME>` with "who · saved · updated" muted at the right of the
  // same line (ellipsised when tight, its full text on hover), its title, its video and up to four screenshot tiles
  // ("+N" on the fourth when there are more), then its notes as message markdown. `wide` is the spec's media row (§4.1):
  // the video left and a 2×2 screenshot grid right; `stacked` puts one under the other, for narrow places. Props only,
  // no store: the build page, the side panel (#619) and the pop-out window (#620) each feed it.
  import type { Snippet } from 'svelte'
  import { renderMarkdown } from '../lib/markdown'
  import type { BuildScreenshot as Screenshot } from '../lib/builds'
  import type { BuildLink } from '../lib/protocol/BuildLink'

  let { title, gameName, guessed = false, authorName, savedAt, updatedAt, notes, screenshots, link, archived, quickDrop = false, onImage, video, layout = 'stacked', videoEnlarged = false }: {
    title: string
    gameName: string | null
    /** The game was picked automatically from the link's title (shown "auto picked"). */
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
    /** The media row: video left and screenshots right (`wide`), or one under the other. */
    layout?: 'wide' | 'stacked'
    /** The video is enlarged: on the wide layout it takes the whole row. */
    videoEnlarged?: boolean
  } = $props()

  const MAX_TILES = 4
  const tiles = $derived(screenshots.slice(0, MAX_TILES))
  const more = $derived(screenshots.length - MAX_TILES)
  const caption = $derived(gameName ? `Builds · ${gameName}` : 'Builds')
  const meta = $derived([authorName, `saved ${savedAt}`, ...(updatedAt ? [`updated ${updatedAt}`] : [])].join(' · '))
  const html = $derived(notes?.trim() ? renderMarkdown(notes) : '')
</script>

<article class="build-view" class:archived data-layout={layout}>
  <header>
    <div class="top">
      <span class="caption">{caption}{#if gameName && guessed}<span class="auto-picked">{' · auto picked'}</span>{/if}</span>
      <span class="meta" data-tip={meta}>{meta}</span>
    </div>
    <h1>{title}</h1>
    {#if archived || quickDrop}
      <span class="badges">
        {#if quickDrop}<span class="badge">QUICK DROP</span>{/if}
        {#if archived}<span class="badge">ARCHIVED</span>{/if}
      </span>
    {/if}
  </header>
  {#if (video && link) || tiles.length}
    <div class="media" class:wide={layout === 'wide'} class:enlarged={videoEnlarged} class:solo={!(video && link)} class:no-tiles={!tiles.length}>
      {#if video && link}<div class="video">{@render video()}</div>{/if}
      {#if tiles.length}
        <div class="tiles" class:one={tiles.length === 1}>
          {#each tiles as s, i (s.id)}
            <svelte:element this={onImage ? 'button' : 'div'} type={onImage ? 'button' : undefined} class="tile" role={onImage ? undefined : 'presentation'}
              aria-label={onImage ? `Open screenshot ${i + 1}${i === MAX_TILES - 1 && more > 0 ? `, and ${more} more` : ''}` : undefined} onclick={onImage ? () => onImage(i) : undefined}
              style:aspect-ratio={layout === 'stacked' && s.w && s.h ? `${s.w} / ${s.h}` : undefined}>
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
  header { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  /* the caption line: the caption wraps; the meta sits at its right when it fits, else on its own line (right-aligned),
     and ellipsises only when that line is too narrow (#618 testing, #645 review: never squeezed to nothing) */
  .top { display: flex; flex-wrap: wrap; align-items: baseline; column-gap: 12px; row-gap: 2px; min-width: 0; }
  .caption { flex: 0 1 auto; min-width: 0; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); overflow-wrap: anywhere; }
  h1 { margin: 0; font-size: 22px; font-weight: 700; color: var(--text); overflow-wrap: anywhere; }
  .meta { flex: 0 1 auto; min-width: 0; margin-left: auto; font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .badges { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .badge { padding: 1px 7px; border-radius: var(--radius-xs); border: 1px solid var(--bg-4); color: var(--text-2); font-size: 10px; font-weight: 700; letter-spacing: .04em; }
  .media { display: flex; flex-direction: column; gap: 8px; }
  /* the spec's media row (§4.1): the video and a 2×2 grid side by side, each at most 360px and both shrinking */
  .media.wide { display: grid; grid-template-columns: repeat(2, minmax(0, 360px)); gap: 12px; align-items: start; }
  .media.wide .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .media.wide .tiles.one { grid-template-columns: minmax(0, 1fr); }
  .media.wide .tile { max-height: none; }
  .media.wide.solo .tiles { grid-column: 1 / -1; }
  /* a video with no screenshots: one column as wide as the player gets (before enlarged, which still takes the row) */
  .media.wide.no-tiles { grid-template-columns: minmax(0, 640px); }
  /* enlarged: the video takes the row, the screenshots a strip under it */
  .media.wide.enlarged { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .media.wide.enlarged .video { grid-column: 1 / -1; }
  .media.wide.enlarged .tiles { grid-column: 1 / -1; grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .video { min-width: 0; }
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
