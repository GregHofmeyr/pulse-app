<script lang="ts">
  // One link's preview card under a message, built only from the server's structured data (text
  // bindings, never {@html}). A player's iframe exists only after a click, only for the one active
  // card app-wide, and only ever loads our own embed page.
  import { onDestroy } from 'svelte'
  import Icon from './Icon.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { aspect, audioHeight, playable, playerSrc, popsOut, providerLabel, sandboxFor } from '../lib/embeds'
  import { embed as origin, loadEmbedOrigin, play, player, playerKey, stop } from '../lib/linkPreviews.svelte'
  import type { LinkEmbed } from '../lib/protocol/LinkEmbed'

  let { embed, messageId, mine, onError = () => {} }: {
    embed: LinkEmbed
    messageId: string
    /** Your own message: you may remove the preview (for everyone). */
    mine: boolean
    /** A failed hide or pop-out, for the message's inline error. */
    onError?: (text: string) => void
  } = $props()

  loadEmbedOrigin()

  // Brand accents for the left bar (fixed per provider, never from the link's data).
  const BRAND: Record<string, string> = {
    youtube: '#ff3344', tiktok: '#fe2c55', spotify: '#1ed760', soundcloud: '#ff5500', twitch: '#9146ff', instagram: '#e1306c',
  }

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

<div class="wrap">
  <div class="embed" class:big class:playing={active} bind:this={root} style:--brand={BRAND[embed.provider] ?? 'var(--bg-4)'}>
    <div class="text">
      <span class="label">{label}{#if embed.author}<span class="by"> · {embed.author}</span>{/if}</span>
      <a class="title" href={embed.url} target="_blank" rel="noopener noreferrer">{embed.title ?? embed.url}</a>
      {#if embed.description}<p class="desc">{embed.description}</p>{/if}
    </div>
    {#if active}
      <div class="frame" class:vertical={shape === '9 / 16'} class:audio={shape === 'audio'}
        style:aspect-ratio={shape === 'audio' ? undefined : shape} style:height={shape === 'audio' ? `${audioHeight(embed)}px` : undefined}>
        <iframe src={playerSrc(origin.origin!, embed)} title={embed.title ? `${label}: ${embed.title}` : `${label} player`}
          sandbox={sandboxFor(embed)} allow="autoplay; encrypted-media; fullscreen; picture-in-picture"
          referrerpolicy="strict-origin-when-cross-origin"></iframe>
      </div>
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
  {#if mine}
    <button class="hide" aria-label="Remove preview" onclick={() => void hide()}><Icon name="x" size={14} /></button>
  {/if}
</div>

<style>
  .wrap { display: flex; align-items: flex-start; gap: 4px; margin-top: 6px; max-width: 460px; }
  .embed {
    position: relative; flex: 0 1 432px; min-width: 0; box-sizing: border-box;
    display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px 14px; align-items: start;
    padding: 10px 12px 12px 14px; border-radius: var(--radius-xs); background: var(--bg-2); border: 1px solid var(--line-soft);
    box-shadow: inset 4px 0 0 var(--brand);
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
  .hide {
    width: 24px; height: 24px; border: 0; border-radius: var(--radius-xs); padding: 0; background: transparent; color: var(--text-3);
    /* its own layer for good, as the entrances (#103): a fade that makes and drops one flashes on WebKitGTK */
    display: grid; place-items: center; opacity: 0; transition: opacity var(--dur-fast) var(--ease); will-change: opacity;
  }
  .wrap:hover .hide, .hide:focus-visible, .wrap:focus-within .hide { opacity: 1; }
  .hide:hover { color: var(--text); background: var(--bg-3); }
</style>
