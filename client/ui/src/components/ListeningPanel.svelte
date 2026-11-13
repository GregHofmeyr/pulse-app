<script lang="ts">
  // "Listening to" on a profile card (#275): the album art (as Spotify gives it: uncropped, 4px corners, nothing on
  // top), the song and artist, Spotify's single-colour icon beside (never over) the art, and the link-out in their
  // approved wording. Nothing here controls anyone's Spotify (#196).
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { artUrl, listenLink } from '../lib/activity'
  import type { Listening } from '../lib/protocol/Listening'

  let { listening }: { listening: Listening } = $props()
  const art = $derived(artUrl(listening))
  const link = $derived(listenLink(listening))
</script>

<section class="listening" aria-label="Listening to {listening.title} on Spotify">
  <div class="head"><span>Listening to Spotify</span><SpotifyIcon /></div>
  <div class="row">
    {#if art}<img class="art" src={art} alt="" width={listening.art!.width} height={listening.art!.height} />{/if}
    <div class="words">
      <span class="title" data-tip={listening.title}>{listening.title}</span>
      {#if listening.artist}<span class="sub" data-tip={listening.artist}>{listening.artist}</span>{/if}
    </div>
  </div>
  <a class="open" href={link.href} target="_blank" rel="noopener noreferrer">{link.label}</a>
</section>

<style>
  /* the card's own translucent panel (ProfileCard .panel), sized with the card (--k, #156) */
  .listening { border-radius: var(--radius-md); padding: calc(10px * var(--k, 1)) calc(12px * var(--k, 1)); background: var(--card-panel); display: flex; flex-direction: column; gap: 8px; font-size: calc(13px * var(--k, 1)); color: var(--card-text); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: calc(12px * var(--k, 1)); font-weight: 600; color: var(--card-text-2); }
  .row { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .art { flex: none; width: calc(48px * var(--k, 1)); height: calc(48px * var(--k, 1)); object-fit: contain; border-radius: 4px; display: block; }
  .words { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--card-text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .open { align-self: flex-start; height: 28px; padding: 0 10px; display: inline-flex; align-items: center; border-radius: var(--radius-sm); border: 1px solid var(--card-edge); color: var(--card-text); font-size: 12px; font-weight: 600; text-decoration: none; }
  .open:hover { background: var(--card-hover); }
</style>
