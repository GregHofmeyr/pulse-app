<script lang="ts">
  // "Listening to" on a profile card (#275): the album art (as Spotify gives it: uncropped, 4px corners, nothing on
  // top), the song and artist, Spotify's single-colour icon beside (never over) the art, and the link-out in their
  // approved wording. Nothing here controls anyone's Spotify (#196).
  // Share to stage (#321): a known song (its Spotify id, so a real link) can go on the stage of the server voice call
  // you're in, through the same share as a Spotify card in chat (#234).
  // Another player (#322): its name as the heading and a generic icon; no link-out, no art, nothing to share.
  import PlayerIcon from './PlayerIcon.svelte'
  import { artUrl, listenLink, playerName, playerOf, sourceLine } from '../lib/activity'
  import { api, errorText } from '../lib/tauri'
  import { app } from '../lib/store.svelte'
  import { voice } from '../lib/voice.svelte'
  import type { Listening } from '../lib/protocol/Listening'

  let { listening }: { listening: Listening } = $props()
  const art = $derived(artUrl(listening))
  const link = $derived(listenLink(listening))

  // the call you're in, when it's a server voice channel (a DM call has no stage)
  const stage = $derived(
    voice.connection === 'connected' && voice.channelId && app.state.channels[voice.channelId]?.kind === 'voice' ? voice.channelId : null,
  )
  // only a song whose id is known: a search link isn't something to share
  const canShare = $derived(!!stage && link?.label === 'Listen on Spotify')
  const onStage = $derived(canShare && (app.state.stageShares[stage!] ?? []).some((x) => x.embed.url === link!.href))
  let sharing = $state(false)
  let error = $state<string | null>(null)
  async function share() {
    if (!stage || !link || sharing) return
    sharing = true
    error = null
    try {
      await api.shareToStage(stage, link.href)
    } catch (e) {
      error = errorText(e)
    } finally {
      sharing = false
    }
  }
</script>

<section class="listening" aria-label="Listening to {listening.title} {listening.player === 'browser' ? 'in' : 'on'} {playerName(playerOf(listening))}">
  <div class="head"><span>{sourceLine(listening)}</span><PlayerIcon {listening} /></div>
  <div class="row">
    {#if art}<img class="art" src={art} alt="" width={listening.art!.width} height={listening.art!.height} />{/if}
    <div class="words">
      <span class="title" data-tip={listening.title}>{listening.title}</span>
      {#if listening.artist}<span class="sub" data-tip={listening.artist}>{listening.artist}</span>{/if}
    </div>
  </div>
  {#if link}<div class="actions">
    <a class="open" href={link.href} target="_blank" rel="noopener noreferrer">{link.label}</a>
    {#if onStage}
      <span class="open on-stage" aria-disabled="true">On the stage</span>
    {:else if canShare}
      <button type="button" class="open" disabled={sharing} data-tip="Show it on the stage for everyone in the call" onclick={() => void share()}>Share to stage</button>
    {/if}
  </div>{/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
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
  .actions { display: flex; flex-wrap: wrap; gap: 6px; }
  .open { background: none; font-family: inherit; cursor: pointer; align-self: flex-start; height: 28px; padding: 0 10px; display: inline-flex; align-items: center; border-radius: var(--radius-sm); border: 1px solid var(--card-edge); color: var(--card-text); font-size: 12px; font-weight: 600; text-decoration: none; }
  .open:hover:not(:disabled):not(.on-stage) { background: var(--card-hover); }
  .open:disabled { opacity: 0.6; cursor: default; }
  .on-stage { cursor: default; color: var(--card-text-2); }
  .err { margin: 0; font-size: 12px; color: var(--danger-text); }
</style>
