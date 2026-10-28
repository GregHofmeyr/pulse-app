<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { displayName, liveIn } from '../lib/selectors'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { clampVolume, volumeLabel } from '../lib/voiceui'
  import { clickOutside } from '../lib/actions'
  import { menuItems, type MenuCtx } from '../lib/participantMenu'
  import { isWindows } from '../lib/stream/platform'
  import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
  import { callView, joinAndWatch, openFull } from '../lib/stream/viewer.svelte'
  import { pickSource } from '../lib/stream/golive.svelte'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { openCard } from '../lib/profileCard.svelte'

  let {
    channelId,
    serverId,
    userId,
    x,
    y,
    opener,
    onClose,
    onMessage,
    onVoiceSettings,
  }: {
    channelId: string
    serverId: string | null
    userId: string
    x: number
    y: number
    /** What had focus (or was clicked): focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onMessage: (userId: string) => void
    onVoiceSettings: () => void
  } = $props()

  const windows = isWindows()
  const self = $derived(userId === app.state.me?.id)
  const name = $derived(displayName(app.state, serverId, userId))
  const live = $derived(!!liveIn(app.state, channelId)[userId])
  const watching = $derived(!!findWatch(channelId, userId))
  const full = $derived(callView.full?.channelId === channelId && callView.full.streamer === userId)
  const inChannel = $derived(voice.channelId === channelId)
  const inCall = $derived(inChannel && voice.connection === 'connected')
  const ctx: MenuCtx = $derived({
    self,
    live,
    watching,
    full,
    inCall,
    canGoLive: inCall && !stream.live,
    streamingSelf: stream.live?.channelId === channelId,
  })
  const ids = $derived(menuItems(ctx))
  const volume = $derived(voice.volumes[userId] ?? 100)
  const streamVolume = $derived(stream.volumes[userId] ?? 100)
  const muted = $derived(!!voice.localMutes[userId])
  const watchReason = $derived(windows ? null : watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId }))

  let el = $state<HTMLElement>()
  // seeded once, then clamped into the window once it has a size
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  let closed = false

  onMount(() => {
    if (!el) return
    const r = el.getBoundingClientRect()
    left = Math.max(8, Math.min(x, window.innerWidth - r.width - 8))
    top = Math.max(8, Math.min(y, window.innerHeight - r.height - 8))
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLElement>('[data-item]')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }

  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back first (so a dialog the action opens remembers the right place), then it acts, then the menu goes:
   *  the action reads this menu's props, which end with it. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }

  function step(input: HTMLInputElement, dir: 1 | -1) {
    const next = clampVolume(Math.round((+input.value + dir * 5) / 5) * 5)
    input.value = String(next)
    if (input.dataset.item === 'stream-volume') streamApi.setStreamVolume(userId, next)
    else void voiceApi.setVolume(userId, next)
  }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusItem(0); break
      case 'End': e.preventDefault(); focusItem(-1); break
      case 'Escape': e.preventDefault(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
      case 'ArrowLeft':
      case 'ArrowRight': {
        const t = e.target as HTMLElement
        if (t instanceof HTMLInputElement) {
          e.preventDefault()
          step(t, e.key === 'ArrowRight' ? 1 : -1)
        }
        break
      }
      case 'Enter': {
        // a button's own Enter would click it too; one path keeps it to a single activation
        const t = e.target as HTMLElement
        if (t instanceof HTMLButtonElement) {
          e.preventDefault()
          t.click()
        }
        break
      }
    }
  }
</script>

<div bind:this={el} class="menu" role="menu" aria-label="Actions for {name}" tabindex="-1" style:position="fixed" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  <!-- the card opens beside the row or tile the menu came from -->
  <button type="button" role="menuitem" data-item="profile" onclick={() => pick(() => openCard(userId, serverId, opener))}>View profile</button>
  {#each ids as id (id)}
    {#if id === 'volume'}
      <div class="slider" role="none">
        <span class="lbl">User volume</span><strong>{volumeLabel(volume)}</strong>
        {#if muted}<span class="hint">Muted for you</span>{/if}
        <input data-item="volume" type="range" min="0" max="200" step="5" value={volume} aria-label="User volume" aria-valuetext={volumeLabel(volume)}
          oninput={(e) => voiceApi.setVolume(userId, clampVolume(+e.currentTarget.value))} />
      </div>
    {:else if id === 'local-mute'}
      <button type="button" role="menuitem" data-item="local-mute" onclick={() => pick(() => voiceApi.setLocalMute(userId, !muted))}>{muted ? 'Unmute for me' : 'Mute for me'}</button>
    {:else if id === 'message'}
      <button type="button" role="menuitem" data-item="message" onclick={() => pick(() => onMessage(userId))}>Message</button>
    {:else if id === 'stream-volume'}
      <div class="slider" role="none">
        <span class="lbl">Stream volume</span><strong>{volumeLabel(streamVolume)}</strong>
        <input data-item="stream-volume" type="range" min="0" max="200" step="5" value={streamVolume} aria-label="Stream volume" aria-valuetext={volumeLabel(streamVolume)}
          oninput={(e) => streamApi.setStreamVolume(userId, +e.currentTarget.value)} />
      </div>
    {:else if id === 'watch'}
      {#if watchReason}
        <button type="button" role="menuitem" data-item="watch" aria-disabled="true" class="off">Watch <small>{watchReason}</small></button>
      {:else if inChannel}
        <button type="button" role="menuitem" data-item="watch" onclick={() => pick(() => streamApi.watch(channelId, userId))}>Watch</button>
      {:else}
        <button type="button" role="menuitem" data-item="watch" onclick={() => pick(() => joinAndWatch(channelId, userId))}>Join &amp; watch</button>
      {/if}
    {:else if id === 'stop-watching'}
      <button type="button" role="menuitem" data-item="stop-watching" onclick={() => pick(() => streamApi.unwatch(channelId, userId))}>Stop watching</button>
    {:else if id === 'full-view'}
      <button type="button" role="menuitem" data-item="full-view" onclick={() => pick(() => { openFull({ channelId, streamer: userId }); callView.goTo = channelId })}>Full view</button>
    {:else if id === 'mute'}
      <button type="button" role="menuitem" data-item="mute" onclick={() => pick(() => voiceApi.toggleMute())}>{voice.controls.muted ? 'Unmute' : 'Mute'}</button>
    {:else if id === 'deafen'}
      <button type="button" role="menuitem" data-item="deafen" onclick={() => pick(() => voiceApi.toggleDeafen())}>{voice.controls.deafened ? 'Undeafen' : 'Deafen'}</button>
    {:else if id === 'go-live'}
      {#if windows}
        <button type="button" role="menuitem" data-item="go-live" onclick={() => pick(() => pickSource(channelId))}>Go Live</button>
      {:else}
        <button type="button" role="menuitem" data-item="go-live" aria-disabled="true" class="off">Go Live <small>Streaming from Linux isn't supported yet</small></button>
      {/if}
    {:else if id === 'stop-streaming'}
      <button type="button" role="menuitem" data-item="stop-streaming" onclick={() => pick(() => streamApi.stop())}>Stop streaming</button>
    {:else if id === 'voice-settings'}
      <button type="button" role="menuitem" data-item="voice-settings" onclick={() => pick(onVoiceSettings)}>Voice settings</button>
    {/if}
  {/each}
</div>

<style>
  .menu { position: fixed; z-index: 60; min-width: 220px; max-width: 280px; padding: 6px; background: #2e3037; border: 1px solid #3d4048; border-radius: 12px; box-shadow: 0 18px 48px rgba(8, 9, 12, .5); display: flex; flex-direction: column; gap: 2px; font-size: 13px; outline: none; }
  .menu button { height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text); text-align: left; font: inherit; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .menu button:hover, .menu button:focus-visible { background: var(--bg-3); }
  .menu button.off { color: var(--text-3); cursor: not-allowed; height: auto; min-height: 32px; flex-direction: column; align-items: flex-start; justify-content: center; padding: 6px 10px; gap: 2px; }
  .menu button.off small { font-size: 11px; }
  .slider { padding: 6px 10px 8px; display: grid; grid-template-columns: 1fr auto; gap: 4px 8px; color: var(--text-2); }
  .slider input { grid-column: 1 / -1; accent-color: var(--accent); }
  .slider input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 4px; }
  .slider strong { color: var(--text); font-weight: 600; }
  .hint { grid-column: 1 / -1; font-size: 11px; color: var(--text-3); }
</style>
