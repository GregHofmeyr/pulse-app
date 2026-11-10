<script lang="ts">
  // One link's preview card under a message, built only from the server's structured data (text
  // bindings, never {@html}). A player's iframe exists only after a click, only for the one active
  // card app-wide, and only ever loads our own embed page.
  import { onDestroy } from 'svelte'
  import Icon from './Icon.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { aspect, audioHeight, JAM_SUBTITLE, playable, playerSrc, popsOut, providerLabel, sandboxFor, spotifyCard } from '../lib/embeds'
  import { embed as origin, loadEmbedOrigin, play, player, playerKey, stop } from '../lib/linkPreviews.svelte'
  import type { LinkEmbed } from '../lib/protocol/LinkEmbed'
  import { app } from '../lib/store.svelte'
  import { voice } from '../lib/voice.svelte'

  let { embed, messageId, channelId = null, mine, onError = () => {} }: {
    embed: LinkEmbed
    messageId: string
    /** The message's chat: in a voice channel whose call you're in, a Spotify card can go on its stage (#234). */
    channelId?: string | null
    /** Your own message: you may remove the preview (for everyone). */
    mine: boolean
    /** A failed hide or pop-out, for the message's inline error. */
    onError?: (text: string) => void
  } = $props()

  loadEmbedOrigin()

  const key = $derived(playerKey(messageId, embed.url))
  const label = $derived(providerLabel(embed))
  const canPlay = $derived(playable(embed))
  const pops = $derived(popsOut(embed))
  const shape = $derived(aspect(embed))
  const active = $derived(canPlay && !pops && !!origin.origin && player.active === key)
  const thumbSrc = $derived(embed.thumb && isMediaHash(embed.thumb.hash) ? mediaUrl(embed.thumb.hash) : null)
  // Video players show a large thumbnail; generic cards, audio and Instagram a small one at the side.
  const big = $derived(canPlay && !pops && shape !== 'audio')
  const hasMedia = $derived(!!thumbSrc || canPlay)
  // Spotify's own card (#196): their attribution and link-out wording, artwork left as it is.
  const sp = $derived(spotifyCard(embed))
  const spTitle = $derived(sp?.jam ? 'Join the Jam' : (embed.title ?? embed.url))
  const spSub = $derived(sp?.jam ? JAM_SUBTITLE : (embed.author ?? embed.description))

  // Share to stage (#234): a Spotify card in the chat of the voice channel you're in a call in.
  const canShare = $derived(!!sp && !!channelId && app.state.channels[channelId]?.kind === 'voice' && voice.channelId === channelId && voice.connection === 'connected')
  let sharing = $state(false)
  // this very link is what's on the stage now: say so instead of offering it again
  const onStage = $derived(canShare && !!channelId && (app.state.stageShares[channelId] ?? []).some((x) => x.embed.url === embed.url))
  async function shareToStage() {
    if (!channelId || sharing) return
    sharing = true
    try {
      await api.shareToStage(channelId, embed.url)
    } catch (e) {
      onError(errorText(e))
    } finally {
      sharing = false
    }
  }

  async function start() {
    if (pops) {
      try {
        await api.openEmbedWindow(embed.provider, embed.provider_id!)
      } catch (e) {
        onError(errorText(e))
      }
      return
    }
    play(key)
  }

  async function hide() {
    try {
      await api.hideEmbed(messageId, embed.url)
    } catch (e) {
      onError(errorText(e))
    }
  }

  // Scrolled far away (or this card went away): the player stops.
  let root: HTMLElement | undefined = $state()
  $effect(() => {
    if (!active || !root || typeof IntersectionObserver !== 'function') return
    const k = key
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => !e.isIntersecting)) stop(k)
    }, { rootMargin: '400px' })
    io.observe(root)
    return () => io.disconnect()
  })
  onDestroy(() => stop(key))
</script>

{#snippet frame()}
  <div class="frame" class:vertical={shape === '9 / 16'} class:audio={shape === 'audio'}
    style:aspect-ratio={shape === 'audio' ? undefined : shape} style:height={shape === 'audio' ? `${audioHeight(embed)}px` : undefined}>
    <iframe src={playerSrc(origin.origin!, embed)} title={embed.title ? `${label}: ${embed.title}` : `${label} player`}
      sandbox={sandboxFor(embed)} allow="autoplay; encrypted-media; fullscreen; picture-in-picture"
      referrerpolicy="strict-origin-when-cross-origin"></iframe>
  </div>
{/snippet}

<div class="wrap">
  {#if sp}
  <div class="embed spotify" class:playing={active} bind:this={root}>
    <div class="text">
      <span class="label sp-label"><SpotifyIcon /><span class="sp-kind">{sp.label ? `Spotify · ${sp.label}` : 'Spotify'}</span></span>
      <a class="title" href={embed.url} target="_blank" rel="noopener noreferrer" data-tip={spTitle}>{spTitle}</a>
      {#if spSub}<p class="sub" data-tip={spSub}>{spSub}</p>{/if}
      <div class="actions">
        {#if canPlay && origin.origin && !active}
          <button class="btn btn-quiet sp-play" onclick={() => play(key)}><Icon name="play" size={14} />Play</button>
        {/if}
        <a class="btn {sp.jam ? 'btn-primary' : 'btn-secondary'} sp-open" href={embed.url} target="_blank" rel="noopener noreferrer">{sp.action}</a>
        {#if canShare}
          {#if onStage}
            <span class="btn btn-quiet sp-share on-stage" aria-disabled="true">On the stage</span>
          {:else}
            <button class="btn btn-quiet sp-share" disabled={sharing} data-tip="Show it on the stage for everyone in the call" onclick={() => void shareToStage()}>Share to stage</button>
          {/if}
        {/if}
      </div>
    </div>
    {#if active}
      {@render frame()}
    {:else}
      <!-- Spotify's artwork as they give it: uncropped, 4px corners, nothing over it -->
      <div class="art" class:tile={!thumbSrc}>
        {#if thumbSrc}
          <img src={thumbSrc} alt="" loading="lazy" width={embed.thumb!.width} height={embed.thumb!.height} />
        {:else}
          <Icon name="headphones" size={28} />
        {/if}
      </div>
    {/if}
  </div>
  {:else}
  <div class="embed" class:big class:playing={active} bind:this={root}>
    <div class="text">
      <span class="label">{label}{#if embed.author}<span class="by"> · {embed.author}</span>{/if}</span>
      <a class="title" href={embed.url} target="_blank" rel="noopener noreferrer">{embed.title ?? embed.url}</a>
      {#if embed.description}<p class="desc">{embed.description}</p>{/if}
    </div>
    {#if active}
      {@render frame()}
    {:else if hasMedia}
      <div class="media" class:vertical={big && shape === '9 / 16'} style:aspect-ratio={big ? shape : undefined}>
        {#if thumbSrc}
          <img src={thumbSrc} alt="" loading="lazy" width={embed.thumb!.width} height={embed.thumb!.height} />
        {:else}
          <div class="ph" aria-hidden="true"><Icon name={pops ? 'image' : 'play'} size={big ? 32 : 22} /></div>
        {/if}
        {#if canPlay}
          {#if pops || origin.origin}
            <button class="play" aria-label="Play {embed.title ?? label}" data-tip={pops ? `Open in a ${label.split(' ')[0]} window` : 'Play'} onclick={() => void start()}>
              <Icon name="play" size={big ? 22 : 14} />
            </button>
          {:else}
            <a class="play" href={embed.url} target="_blank" rel="noopener noreferrer" aria-label="Play {embed.title ?? label} (opens in your browser)" data-tip="Open in your browser">
              <Icon name="play" size={big ? 22 : 14} />
            </a>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
  {/if}
  {#if mine}
    <button class="hide" aria-label="Remove preview" onclick={() => void hide()}><Icon name="x" size={14} /></button>
  {/if}
</div>

<style>
  .wrap { display: flex; align-items: flex-start; gap: 4px; margin-top: 6px; max-width: 460px; }
  .embed {
    position: relative; flex: 0 1 432px; min-width: 0; box-sizing: border-box;
    display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px 14px; align-items: start;
    padding: 10px 12px 12px; border-radius: var(--radius-xs); background: var(--bg-2); border: 1px solid var(--line-soft);
  }
  .text { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .label { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .by { color: var(--text-3); }
  .title {
    color: var(--accent-text); font-weight: 600; font-size: 14px; line-height: 1.35; text-decoration: none; overflow-wrap: anywhere;
    display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden;
  }
  .title:hover { text-decoration: underline; }
  .desc {
    margin: 0; font-size: 13px; line-height: 1.4; color: var(--text-2); overflow-wrap: anywhere;
    display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden;
  }
  /* generic cards, audio and Instagram: a small thumbnail at the side */
  .media { position: relative; width: 80px; height: 80px; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-3); }
  .media img { width: 100%; height: 100%; object-fit: cover; display: block; }
  /* video players: the thumbnail fills the card's width, at the player's shape, so the swap doesn't jump */
  .big .media, .frame { grid-column: 1 / -1; width: 100%; height: auto; }
  .big .media.vertical, .frame.vertical { width: min(300px, 100%); }
  .frame { border-radius: var(--radius-xs); overflow: hidden; background: #000; }
  .frame.audio { background: transparent; }
  .frame iframe { display: block; width: 100%; height: 100%; border: 0; }
  .ph {
    width: 100%; height: 100%; display: grid; place-items: center; color: var(--text-3);
    background: linear-gradient(135deg, var(--bg-3), var(--bg-4));
  }
  .play {
    position: absolute; inset: 0; margin: auto; width: 34px; height: 34px; border-radius: 50%; border: 0;
    display: grid; place-items: center; color: #fff; background: rgba(10, 11, 13, .62); padding: 0 0 0 2px;
    box-shadow: 0 2px 10px rgba(0, 0, 0, .35); transition: transform var(--dur) var(--ease), background var(--dur) var(--ease);
  }
  .big .play { width: 56px; height: 56px; padding-left: 3px; }
  .play:hover, .play:focus-visible { background: rgba(10, 11, 13, .82); transform: scale(1.06); }
  .play :global(svg) { fill: currentColor; }
  /* Spotify (#196): a row of the icon and the kind, the title, the artist or show, and the actions */
  .sp-label { display: flex; align-items: center; gap: 6px; color: var(--text-2); }
  .sp-kind { overflow: hidden; text-overflow: ellipsis; }
  .sub {
    margin: 0; font-size: 13px; line-height: 1.4; color: var(--text-2); overflow-wrap: anywhere;
    display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden;
  }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
  .actions .btn { height: 28px; padding: 0 10px; font-size: 12px; text-decoration: none; }
  .sp-play :global(svg) { fill: currentColor; }
  .on-stage { cursor: default; color: var(--text-3); }
  /* Spotify's rules: 4px corners, never cropped (contain), nothing on top */
  .art { width: 96px; height: 96px; border-radius: 4px; overflow: hidden; display: grid; place-items: center; }
  .art img { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; display: block; border-radius: 4px; }
  .art.tile { color: var(--text-3); background: linear-gradient(135deg, var(--bg-3), var(--bg-4)); }
  .spotify .frame { grid-column: 1 / -1; }
  .hide {
    width: 24px; height: 24px; border: 0; border-radius: var(--radius-xs); padding: 0; background: transparent; color: var(--text-3);
    /* its own layer for good, as the entrances (#103): a fade that makes and drops one flashes on WebKitGTK */
    display: grid; place-items: center; opacity: 0; transition: opacity var(--dur-fast) var(--ease); will-change: opacity;
  }
  .wrap:hover .hide, .hide:focus-visible, .wrap:focus-within .hide { opacity: 1; }
  .hide:hover { color: var(--text); background: var(--bg-3); }
</style>
