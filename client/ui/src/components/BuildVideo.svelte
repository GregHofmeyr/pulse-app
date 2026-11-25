<script lang="ts">
  // A build's link in its page's media area (#618, spec §4.2). A YouTube video plays in place through the same player
  // path as a link preview (LinkEmbed: our embed page, its sandbox, the one-player-at-a-time `player`); leaving the page
  // stops it. Its controls sit on the player's top-right as a video player's do (#618 testing): Enlarge (fills the
  // media area) and, while playing, Close player, shown on hover and whenever Tab reaches them. Under it, the video's
  // title (plain, ellipsised) and a small "Open on YouTube" button. Anything else (a build planner, another site's
  // video) is a card that opens in the browser.
  import { onDestroy } from 'svelte'
  import Icon from './Icon.svelte'
  import { linkAsEmbed, playable, playerSrc, popsOut, providerLabel, sandboxFor, YOUTUBE_FRAMING_ERROR } from '../lib/embeds'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { embed as origin, fallBack, fallen, loadEmbedOrigin, play, player, playerKey, stop } from '../lib/linkPreviews.svelte'
  import { safeHref } from '../lib/safeHref'
  import { api, errorText } from '../lib/tauri'
  import type { BuildLink } from '../lib/protocol/BuildLink'

  let { link, messageId, enlarged = $bindable(false), onError = () => {} }: {
    link: BuildLink
    /** The build's id (the player's key, with the link). */
    messageId: string
    /** Fills the media area. */
    enlarged?: boolean
    onError?: (text: string) => void
  } = $props()

  loadEmbedOrigin()

  const e = $derived(linkAsEmbed(link))
  const key = $derived(playerKey(messageId, link.url))
  const video = $derived(playable(e))
  const pops = $derived(popsOut(e, fallen.providers))
  const active = $derived(video && !pops && !!origin.origin && player.active === key)
  const thumbSrc = $derived(link.thumb && isMediaHash(link.thumb.hash) ? mediaUrl(link.thumb.hash) : null)
  const title = $derived(link.title ?? link.url)
  const provider = $derived(providerLabel(e))
  const host = $derived.by(() => {
    try {
      return new URL(link.url).hostname.replace(/^www\./, '')
    } catch {
      return ''
    }
  })

  async function popOut() {
    try {
      await api.openEmbedWindow(e.provider, e.provider_id!, null)
    } catch (err) {
      onError(errorText(err))
    }
  }
  function start() {
    if (pops) return void popOut()
    play(key)
  }
  onDestroy(() => stop(key))

  // Our embed page says the player ended or failed: back to the thumbnail. YouTube refusing to play framed (#88) opens
  // its videos in the pop-out window from now on, as a preview card does.
  let frameEl: HTMLIFrameElement | undefined = $state()
  $effect(() => {
    if (!active || !origin.origin) return
    const from = new URL(origin.origin).origin
    const k = key
    const onMessage = (m: MessageEvent) => {
      if (m.origin !== from || !frameEl || m.source !== frameEl.contentWindow) return
      const d: unknown = m.data
      if (!d || typeof d !== 'object' || (d as { pulse?: unknown }).pulse !== 'player') return
      const { state, code } = d as { state?: unknown; code?: unknown }
      if (state === 'error' && code === YOUTUBE_FRAMING_ERROR) {
        fallBack('youtube')
        stop(k)
        void popOut()
      } else if (state === 'ended' || state === 'error') stop(k)
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  })
</script>

{#if video}
  <div class="build-video" class:enlarged>
    <div class="screen">
      {#if active}
        <iframe bind:this={frameEl} src={playerSrc(origin.origin!, e)} title={`${provider}: ${title}`} sandbox={sandboxFor(e)}
          allow="autoplay; encrypted-media; fullscreen; picture-in-picture" referrerpolicy="strict-origin-when-cross-origin"></iframe>
      {:else}
        {#if thumbSrc}<img src={thumbSrc} alt="" loading="lazy" />{:else}<div class="ph" aria-hidden="true"><Icon name="film" size={32} /></div>{/if}
        {#if pops || origin.origin}
          <button type="button" class="play" aria-label="Play {title}" data-tip={pops ? 'Open in a YouTube window' : 'Play'} onclick={start}><Icon name="play" size={22} /></button>
        {:else}
          <a class="play" href={safeHref(link.url)} target="_blank" rel="noopener noreferrer" aria-label="Play {title} (opens in your browser)"><Icon name="play" size={22} /></a>
        {/if}
      {/if}
      <div class="corner">
        {#if active}
          <button type="button" class="icon-btn icon-btn-sm icon-btn-overlay stop" aria-label="Close player" data-tip="Close player" onclick={() => stop(key)}><Icon name="x" size={14} /></button>
        {/if}
        <button type="button" class="icon-btn icon-btn-sm icon-btn-overlay enlarge" aria-label="Enlarge video" aria-pressed={enlarged} data-tip={enlarged ? 'Shrink' : 'Enlarge'}
          onclick={() => (enlarged = !enlarged)}><Icon name={enlarged ? 'min' : 'expand'} size={14} /></button>
      </div>
    </div>
    <div class="under">
      {#if link.title}<span class="yt-title" data-tip={link.title}>{link.title}</span>{/if}
      <a class="open-yt" href={safeHref(link.url)} target="_blank" rel="noopener noreferrer" aria-label="Open on {provider} (opens in your browser)">Open on {provider} <span aria-hidden="true">↗</span></a>
    </div>
  </div>
{:else}
  <a class="link-card" href={safeHref(link.url)} target="_blank" rel="noopener noreferrer">
    <span class="thumb">{#if thumbSrc}<img src={thumbSrc} alt="" loading="lazy" />{:else}<Icon name="unlink" size={20} />{/if}</span>
    <span class="text">
      <span class="title">{title}</span>
      {#if host}<span class="host">{host} · opens in your browser</span>{/if}
    </span>
  </a>
{/if}

<style>
  .build-video { display: flex; flex-direction: column; gap: 6px; max-width: 640px; }
  .build-video.enlarged { max-width: none; }
  .screen { position: relative; aspect-ratio: 16 / 9; border-radius: var(--radius-md); overflow: hidden; background: var(--bg-3); }
  .screen img, .screen iframe { position: absolute; inset: 0; width: 100%; height: 100%; border: 0; object-fit: cover; }
  .ph { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: var(--text-3); }
  .play { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 56px; height: 56px; border-radius: 50%; border: 0; display: flex; align-items: center; justify-content: center; background: var(--scrim-strong); color: var(--tooltip-text); cursor: pointer; }
  .play:hover { filter: brightness(1.15); }
  /* the player's own controls, top-right (#618 testing): shown on hover, when Tab reaches one, and always on touch */
  .corner { position: absolute; z-index: 1; top: 8px; right: 8px; display: flex; gap: 4px; opacity: 0; transition: opacity var(--dur-fast) var(--ease); }
  .build-video:hover .corner, .corner:focus-within { opacity: 1; }
  @media (hover: none) { .corner { opacity: 1; } }
  .under { display: flex; align-items: center; gap: 8px; min-width: 0; min-height: 24px; }
  .yt-title { flex: 1; min-width: 0; font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .open-yt { flex-shrink: 0; margin-left: auto; display: inline-flex; align-items: center; gap: 4px; height: 24px; padding: 0 8px; box-sizing: border-box; border: 1px solid var(--bg-3); border-radius: var(--radius-xs); background: var(--bg-2); color: var(--text-2); font-size: 12px; font-weight: 600; white-space: nowrap; text-decoration: none; }
  .open-yt:hover { background: var(--bg-3); color: var(--text); }
  .link-card { display: flex; align-items: center; gap: 12px; max-width: 520px; padding: 10px; border-radius: var(--radius-md); border: 1px solid var(--bg-3); background: var(--bg-2); color: inherit; text-decoration: none; }
  .link-card:hover { border-color: var(--bg-4); }
  .link-card .thumb { flex-shrink: 0; width: 96px; aspect-ratio: 16 / 9; border-radius: var(--radius-sm); overflow: hidden; background: var(--bg-3); display: flex; align-items: center; justify-content: center; color: var(--text-3); }
  .link-card .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .link-card .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .link-card .title { font-size: 14px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link-card .host { font-size: 12px; color: var(--text-3); }
</style>
