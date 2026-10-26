<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import Select from './Select.svelte'
  import UserTile from './UserTile.svelte'
  import { app } from '../lib/store.svelte'
  import { callRoster, displayName, liveIn } from '../lib/selectors'
  import { selfVoice } from '../lib/voice.svelte'
  import { volumeLabel } from '../lib/voiceui'
  import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
  import { hostVideo } from '../lib/stream/videoHost'
  import { closeFull } from '../lib/stream/viewer.svelte'

  // The parent keys this on the stream, so the props never change for one instance.
  let { channelId, streamer, serverId }: { channelId: string; streamer: string; serverId: string | null } = $props()

  const watch = $derived(findWatch(channelId, streamer))
  const name = $derived(displayName(app.state, serverId, streamer))
  const reconnecting = $derived(liveIn(app.state, channelId)[streamer]?.status === 'reconnecting')
  const roster = $derived(callRoster(app.state, serverId, channelId, selfVoice()))
  const volume = $derived(stream.volumes[streamer] ?? 100)
  let quality = $state<string | null>(untrack(() => findWatch(channelId, streamer)?.quality ?? 'auto'))

  // Shown here: the full layer (under Auto) and the diagnostics, also when coming back from the
  // mini-preview. Leaving sets the preview again: Back (`closeFull`) or the mini-preview, not a teardown
  // here (state read in a teardown is the old value).
  $effect(() => {
    untrack(() => streamApi.setView(channelId, streamer, 'full'))
  })

  function stop() {
    // first: closing full view unmounts this, and the props go with it
    void streamApi.unwatch(channelId, streamer)
    closeFull()
  }

  /** Esc goes back to the grid, unless it's meant for a text field, or something open over this view. */
  const TYPING = 'textarea, select, [contenteditable="true"], [role="menu"], [role="listbox"], input:not([type="range"], [type="checkbox"], [type="radio"], [type="button"])'
  const OVER = '[role="dialog"], [aria-modal="true"], [role="menu"], [role="listbox"]'
  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    if (document.activeElement?.closest(TYPING)) return
    // a dialog, popover or menu over this view takes Esc first (its own listener may run after this one)
    if (document.querySelector(OVER)) return
    closeFull()
  }
  // The participant menu arrives with its own task; until then a right-click in the strip does nothing.
  const noMenu = () => {}
</script>

<svelte:window onkeydown={key} />

<div class="full-view">
  <div class="bar">
    <button type="button" class="back" onclick={closeFull}><Icon name="chevronLeft" size={16} /> Back</button>
    <strong class="name">{name}</strong>
    <span class="spacer"></span>
    <div class="ctl">
      <span>Quality</span>
      <Select bind:value={quality} label="Quality" options={[{ value: 'auto', label: 'Auto' }, { value: 'saver', label: 'Data saver' }]}
        onchange={() => streamApi.setQuality(channelId, streamer, quality === 'saver' ? 'saver' : 'auto')} />
    </div>
    <label class="ctl">
      Volume <strong>{volumeLabel(volume)}</strong>
      <input type="range" min="0" max="200" step="5" value={volume} aria-label="Stream volume" oninput={(e) => streamApi.setStreamVolume(streamer, +e.currentTarget.value)} />
    </label>
    <button type="button" class="stop" onclick={stop}>Stop watching</button>
  </div>
  <div class="stage">
    {#if watch}<div class="host" {@attach hostVideo(channelId, streamer)}></div>{/if}
    <StreamDiagOverlay kind="in" />
    {#if reconnecting}<div class="overlay" role="status">Reconnecting…</div>{/if}
  </div>
  <div class="strip">
    {#each roster as entry (entry.userId)}
      <div class="slot"><UserTile {entry} {channelId} compact onMenu={noMenu} /></div>
    {/each}
  </div>
</div>

<style>
  .full-view { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; background: #000; }
  .bar { flex-shrink: 0; height: 52px; padding: 0 16px; display: flex; align-items: center; gap: 14px; background: var(--bg-1); border-bottom: 1px solid var(--bg-3); }
  .back { height: 32px; padding: 0 12px 0 8px; border: 0; border-radius: 10px; background: var(--bg-3); color: var(--text); font-weight: 600; display: flex; align-items: center; gap: 4px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spacer { flex: 1; }
  .ctl { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); }
  input[type='range'] { width: 120px; accent-color: var(--accent); }
  .stop { height: 32px; padding: 0 14px; border: 0; border-radius: 10px; background: var(--danger); color: #fff; font-weight: 700; }
  .stage { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 18px; font-weight: 600; }
  .strip { flex-shrink: 0; padding: 8px 12px; display: flex; gap: 8px; overflow-x: auto; background: var(--bg-1); border-top: 1px solid var(--bg-3); }
  .slot { flex: 0 0 120px; }
</style>
