<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import LiveCard from './LiveCard.svelte'
  import Select from './Select.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName, liveIn } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'
  import { volumeLabel } from '../lib/voiceui'
  import { voice } from '../lib/voice.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
  import { hostVideo } from '../lib/stream/videoHost'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { joinAndWatch } from '../lib/stream/viewer.svelte'

  let {
    channelId,
    serverId,
    streamer,
    mine,
    onMenu,
    onFull,
  }: {
    channelId: string
    serverId: string | null
    streamer: string
    mine: boolean
    onMenu: (e: MouseEvent | KeyboardEvent, anchor: HTMLElement) => void
    onFull: (k: { channelId: string; streamer: string }) => void
  } = $props()

  const windows = isWindows()
  const name = $derived(displayName(app.state, serverId, streamer))
  const info = $derived(liveIn(app.state, channelId)[streamer])
  const viewers = $derived(info?.viewers.length ?? 0)
  const watch = $derived(findWatch(channelId, streamer))
  const reason = $derived(watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId }))
  const inChannel = $derived(voice.channelId === channelId)
  let tile = $state<HTMLElement>()
  // seeded once: the grid keys tiles by streamer, so these props never change for a tile
  const volume = $derived(stream.volumes[streamer] ?? 100)
  let quality = $state<string | null>(untrack(() => findWatch(channelId, streamer)?.quality ?? 'auto'))

  // A click anywhere on a not-watching tile watches; the Watch button is the keyboard route into the same click.
  function clicked() {
    if (watch || reason) return
    void streamApi.watch(channelId, streamer)
  }
  function hostKey(e: KeyboardEvent) {
    if (e.key !== 'Enter' && e.key !== ' ') return
    e.preventDefault()
    onFull({ channelId, streamer })
  }
  function contextMenu(e: MouseEvent) {
    e.preventDefault()
    if (tile) onMenu(e, tile)
  }
  // the keyboard route: the menu key or Shift+F10 on anything focused in the tile; focus returns to that control
  function menuKey(e: KeyboardEvent) {
    if (e.key !== 'ContextMenu' && !(e.key === 'F10' && e.shiftKey)) return
    e.preventDefault()
    const from = e.target instanceof HTMLElement && tile?.contains(e.target) ? e.target : tile
    if (from) onMenu(e, from)
  }
</script>

{#if mine}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tile own" data-stream={streamer} bind:this={tile} tabindex="-1" oncontextmenu={contextMenu} onkeydown={menuKey}>
    <LiveCard {channelId} {serverId} tile />
    <!-- not while our own Stop waits for the server's list to drop us -->
    {#if !stream.live && stream.stopping !== channelId}<p class="reason">You're live from another device</p>{/if}
  </div>
{:else if watch}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_static_element_interactions -->
  <div class="tile watching" data-stream={streamer} bind:this={tile} tabindex="-1" oncontextmenu={contextMenu} onkeydown={menuKey}>
    <div class="host" role="button" tabindex="0" aria-label="Open {name}'s stream in full view" onclick={() => onFull({ channelId, streamer })} onkeydown={hostKey} {@attach hostVideo(channelId, streamer)}></div>
    {#if info?.status === 'reconnecting'}<div class="overlay" role="status">Reconnecting…</div>{/if}
    <div class="name-tag">{name}</div>
    <div class="toolbar">
      <label class="vol">
        <Icon name="speaker" size={14} label="Stream volume" />
        <input type="range" min="0" max="200" step="5" value={volume} aria-label="Stream volume" oninput={(e) => streamApi.setStreamVolume(streamer, +e.currentTarget.value)} />
        <span class="pct">{volumeLabel(volume)}</span>
      </label>
      <div class="quality">
        <Select bind:value={quality} label="Quality" drop="up" options={[{ value: 'auto', label: 'Auto' }, { value: 'saver', label: 'Data saver' }]}
          onchange={() => streamApi.setQuality(channelId, streamer, quality === 'saver' ? 'saver' : 'auto')} />
      </div>
      <span class="spacer"></span>
      <button type="button" class="tb" onclick={() => onFull({ channelId, streamer })}>Full view</button>
      <button type="button" class="tb stop" onclick={() => void streamApi.unwatch(channelId, streamer)}>Stop watching</button>
    </div>
  </div>
{:else}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
  <div class="tile idle" class:blocked={!!reason} data-stream={streamer} bind:this={tile} tabindex={windows ? -1 : 0} onclick={clicked} oncontextmenu={contextMenu} onkeydown={menuKey}>
    <div class="backdrop" style:background={avatarColor(streamer)}></div>
    <span class="live-tag">LIVE</span>
    {#if viewers > 0}<span class="viewers"><Icon name="eye" size={14} label="Viewers" /> {viewers}</span>{/if}
    <span class="av" style:background={avatarColor(streamer)}>{initial(name)}</span>
    <div class="who">{name}</div>
    {#if windows && inChannel}
      <button type="button" class="watch" aria-label="Watch {name}'s stream">Watch</button>
    {:else if windows}
      <!-- joins that voice channel (leaving yours), then opens the stream in full view -->
      <button type="button" class="watch" aria-label="Join and watch {name}'s stream" onclick={() => joinAndWatch(channelId, streamer)}>Join &amp; watch</button>
    {:else}
      <p class="reason">{reason}</p>
    {/if}
  </div>
{/if}

<style>
  .tile { position: relative; aspect-ratio: 16 / 9; min-width: 0; border-radius: 16px; overflow: hidden; background: var(--bg-1); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; }
  .tile:focus { outline: none; }
  .idle { cursor: pointer; }
  .idle.blocked { cursor: default; }
  .backdrop { position: absolute; inset: 0; opacity: .22; }
  .idle > :global(*:not(.backdrop)) { position: relative; }
  .live-tag { position: absolute; top: 10px; left: 12px; height: 18px; padding: 0 7px; border-radius: 5px; background: var(--danger); color: #fff; font-size: 11px; font-weight: 800; letter-spacing: .06em; line-height: 18px; }
  .viewers { position: absolute; top: 10px; right: 12px; display: flex; align-items: center; gap: 4px; font-size: 12px; color: var(--text-2); }
  .av { width: 64px; height: 64px; border-radius: 50%; color: #fff; font-size: 26px; font-weight: 700; display: grid; place-items: center; }
  .who { font-weight: 600; }
  .watch { height: 34px; padding: 0 18px; border: 0; border-radius: 10px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .reason { margin: 0; font-size: 12px; color: var(--text-2); text-align: center; padding: 0 12px; }
  .watching { background: #000; }
  .host { position: absolute; inset: 0; cursor: pointer; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 16px; font-weight: 600; pointer-events: none; }
  .name-tag { position: absolute; left: 10px; bottom: 8px; padding: 2px 8px; border-radius: 6px; background: rgba(0, 0, 0, .55); color: #fff; font-size: 12px; font-weight: 600; pointer-events: none; transition: opacity .12s; }
  .toolbar { position: absolute; left: 0; right: 0; bottom: 0; padding: 8px 10px; display: flex; align-items: center; gap: 10px; background: linear-gradient(transparent, rgba(0, 0, 0, .75)); opacity: 0; transition: opacity .12s; }
  .watching:hover .toolbar, .watching:focus-within .toolbar { opacity: 1; }
  .watching:hover .name-tag, .watching:focus-within .name-tag { opacity: 0; }
  .spacer { flex: 1; }
  .vol { display: flex; align-items: center; gap: 6px; color: #fff; font-size: 12px; }
  .vol input { width: 90px; accent-color: var(--accent); }
  .pct { min-width: 38px; }
  .quality { width: 120px; }
  .quality :global(.trigger) { height: 30px; font-size: 12px; }
  .tb { height: 30px; padding: 0 12px; border: 0; border-radius: 8px; background: rgba(255, 255, 255, .16); color: #fff; font-size: 12px; font-weight: 600; }
  .tb.stop { background: var(--danger); }
</style>
