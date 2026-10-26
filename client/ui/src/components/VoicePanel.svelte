<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { selfVoice, voice, voiceApi } from '../lib/voice.svelte'
  import { callRoster, liveIn } from '../lib/selectors'
  import { gridTiles, nobodyStreaming } from '../lib/callgrid'
  import { stageLabel } from '../lib/voicestage'
  import GoLiveDialog from './GoLiveDialog.svelte'
  import StreamTile from './StreamTile.svelte'
  import UserTile from './UserTile.svelte'
  import { loadShowEveryone, saveShowEveryone } from '../lib/stream/prefs'
  import { StreamRoom, type Picked } from '../lib/stream/room'
  import { isWindows } from '../lib/stream/platform'
  import { stream } from '../lib/stream/stream.svelte'
  import { errorText } from '../lib/tauri'

  let { channelId, serverId }: { channelId: string; serverId: string | null } = $props()

  const channel = $derived(app.state.channels[channelId])
  const inHere = $derived(voice.channelId === channelId)
  const meId = $derived(app.state.me?.id ?? '')
  const roster = $derived(callRoster(app.state, serverId, channelId, selfVoice()))
  let showEveryone = $state(true)
  // the choice is per signed-in user: load it when the user is known (and again if it changes)
  $effect(() => {
    showEveryone = loadShowEveryone(meId)
  })
  let volumeFor = $state<string | null>(null)
  const myLive = $derived(stream.live?.channelId === channelId)
  const tiles = $derived(gridTiles({ roster, streams: liveIn(app.state, channelId), meId, showEveryone, myLive }))

  function setShowEveryone(on: boolean) {
    showEveryone = on
    if (meId) saveShowEveryone(meId, on)
  }
  // The participant menu arrives with its own task; until then a right-click or menu key does nothing here.
  const noMenu = () => {}
  // Full view is wired by the viewer task.
  const noFull = () => {}

  let picked = $state<Picked | null>(null)
  let pickError = $state('')
  const inCall = $derived(inHere && voice.connection === 'connected')
  const canGoLive = $derived(inCall && !stream.live)
  const windows = isWindows()

  async function pickSource() {
    if (!windows) return
    pickError = ''
    try {
      picked = await StreamRoom.pick()
    } catch (e) {
      // cancelling the picker is not an error
      if ((e as Error)?.name !== 'NotAllowedError') pickError = errorText(e)
    }
  }

  function closeDialog() {
    picked = null
  }

  // Leaving voice (or losing the connection) while the dialog is open: drop the picked capture.
  $effect(() => {
    if (picked && !inCall) {
      picked.track.stop()
      picked = null
    }
  })
</script>

<div class="panel">
  <div class="head">
    <Icon name="speaker" /> <span class="title">{channel?.name}</span>
    {#if inHere}
      <span class="status" class:warn={voice.stage.stage !== 'connected'}>
        {stageLabel(voice.stage)}
      </span>
    {/if}
    <button type="button" class="eye" aria-pressed={showEveryone} aria-label="Show non-streaming participants" data-tip="Show non-streaming participants" onclick={() => setShowEveryone(!showEveryone)}>
      <Icon name={showEveryone ? 'eye' : 'eyeOff'} size={18} />
    </button>
  </div>

  {#if voice.error}<p class="error" role="alert">{voice.error}</p>{/if}
  {#if voice.stalled && inHere}<p class="error" role="alert">Your audio device stopped responding. Check it's connected, then rejoin.</p>{/if}

  <div class="grid-wrap">
    {#if nobodyStreaming(tiles, showEveryone)}
      <p class="empty">Nobody's streaming — <button type="button" class="link" onclick={() => setShowEveryone(true)}>show everyone</button></p>
    {:else}
      <div class="grid">
        {#each tiles as t (t.kind === 'user' ? `u:${t.entry.userId}` : `s:${t.streamer}`)}
          {#if t.kind === 'user'}
            <UserTile entry={t.entry} {channelId} onMenu={noMenu} volumeOpen={volumeFor === t.entry.userId}
              onVolume={(open) => (volumeFor = open ? t.entry.userId : null)} />
          {:else}
            <StreamTile {channelId} {serverId} streamer={t.streamer} mine={t.mine} onMenu={noMenu} onFull={noFull} />
          {/if}
        {:else}
          <p class="empty">Nobody's here yet.</p>
        {/each}
      </div>
    {/if}
  </div>

  {#if pickError}<p class="error" role="alert">{pickError}</p>{/if}

  <div class="bar-wrap">
    {#if inHere}
      <div class="controls">
        <button class="ctl" class:on={voice.controls.muted} aria-pressed={voice.controls.muted} aria-label={voice.controls.muted ? "Unmute" : "Mute"} onclick={() => voiceApi.toggleMute()}>
          <Icon name={voice.controls.muted ? 'micOff' : 'mic'} size={20} />
        </button>
        <button class="ctl" class:on={voice.controls.deafened} aria-pressed={voice.controls.deafened} aria-label={voice.controls.deafened ? "Undeafen" : "Deafen"} onclick={() => voiceApi.toggleDeafen()}>
          <Icon name={voice.controls.deafened ? 'headphonesOff' : 'headphones'} size={20} />
        </button>
        {#if canGoLive}
          <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the tooltip -->
          <button class="golive" class:off={!windows} aria-disabled={!windows} data-tip={windows ? undefined : "Streaming from Linux isn't supported yet"} onclick={pickSource}>Go Live</button>
        {/if}
        <button class="leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={22} /></button>
      </div>
    {:else}
      <button class="primary" onclick={() => voiceApi.join(channelId)}>Join voice</button>
    {/if}
  </div>
</div>

{#if picked}<GoLiveDialog {channelId} bind:picked onClose={closeDialog} />{/if}

<style>
  .panel { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--ok); }
  .title { color: var(--text); font-size: 15px; font-weight: 600; flex: 1; }
  .status { font-size: 12px; font-weight: 600; color: var(--ok); padding: 4px 10px; border-radius: 12px; background: rgba(79, 209, 139, .12); }
  .status.warn { color: #e8b04a; background: rgba(232, 176, 74, .12); }
  .error { margin: 12px 18px 0; color: #f2616b; font-size: 13px; }
  .eye { width: 32px; height: 32px; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .eye:hover { background: var(--bg-2); color: var(--text); }
  .eye[aria-pressed='true'] { color: var(--text); }
  .grid-wrap { flex: 1; min-height: 0; overflow-y: auto; container-type: inline-size; }
  .grid { padding: 20px; display: grid; grid-template-columns: 1fr; align-items: start; gap: 14px; }
  @container (min-width: 560px) { .grid { grid-template-columns: repeat(2, 1fr); } }
  @container (min-width: 900px) { .grid { grid-template-columns: repeat(3, 1fr); } }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; }
  .empty { margin: 20px; color: var(--text-3); }
  .bar-wrap { height: 84px; flex-shrink: 0; display: grid; place-items: center; }
  .controls { padding: 8px; background: #17181c; border-radius: 18px; display: flex; gap: 8px; }
  .ctl { width: 48px; height: 48px; border: 0; border-radius: 14px; background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
  .ctl.on { background: var(--danger); color: #fff; }
  .leave { width: 64px; height: 48px; border: 0; border-radius: 14px; background: var(--danger); color: #fff; display: grid; place-items: center; }
  .golive { height: 48px; padding: 0 16px; border: 0; border-radius: 14px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .golive.off { opacity: .5; cursor: not-allowed; }
  .primary { height: 44px; padding: 0 22px; border: 0; border-radius: 12px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
</style>
