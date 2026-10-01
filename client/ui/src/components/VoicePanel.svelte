<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { voiceOccupants } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'
  import { clampVolume, volumeLabel } from '../lib/voiceui'

  let { channelId, serverId }: { channelId: string; serverId: string | null } = $props()

  const channel = $derived(app.state.channels[channelId])
  const inHere = $derived(voice.channelId === channelId)
  const people = $derived(voiceOccupants(app.state, serverId, channelId))
  let popoverFor = $state<string | null>(null)

  const qualityBars = (q: string | undefined) => (q === 'excellent' ? 3 : q === 'good' ? 2 : q === 'poor' ? 1 : q === 'lost' ? 0 : 3)
</script>

<div class="panel">
  <div class="head">
    <Icon name="speaker" /> <span class="title">{channel?.name}</span>
    {#if inHere}
      <span class="status" class:warn={voice.connection !== 'connected'}>
        {voice.connection === 'connected' ? 'Connected' : voice.connection === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'}
      </span>
    {/if}
  </div>

  {#if voice.error}<p class="error" role="alert">{voice.error}</p>{/if}
  {#if voice.stalled && inHere}<p class="error" role="alert">Your audio device stopped responding. Check it's connected, then rejoin.</p>{/if}

  <div class="grid">
    {#each people as p (p.userId)}
      {@const isMe = p.userId === app.state.me?.id}
      <div class="tile" class:speaking={voice.speaking.has(p.userId)}>
        <button class="avatar-btn" disabled={isMe} aria-label={isMe ? p.name : `Volume for ${p.name}`} data-tip={isMe ? undefined : `Adjust ${p.name}'s volume`}
          onclick={() => (popoverFor = popoverFor === p.userId ? null : p.userId)}>
          <span class="av" style:background={avatarColor(p.userId)}>{initial(p.name)}</span>
          {#if p.deafened}<span class="badge"><Icon name="headphonesOff" size={14} label="Deafened" /></span>
          {:else if p.muted}<span class="badge"><Icon name="micOff" size={14} label="Muted" /></span>{/if}
        </button>
        <div class="name">
          {p.name}
          <span class="bars" aria-label="Connection {voice.quality[p.userId] ?? 'good'}">
            {#each [1, 2, 3] as b}<span class="bar" class:on={b <= qualityBars(voice.quality[p.userId])} style:height="{b * 4}px"></span>{/each}
          </span>
        </div>
        {#if (voice.volumes[p.userId] ?? 100) !== 100}<span class="vol">{volumeLabel(voice.volumes[p.userId])}</span>{/if}
        {#if popoverFor === p.userId}
          <div class="pop" role="dialog" aria-label="Volume for {p.name}">
            <label for="vol-{p.userId}">User volume <strong>{volumeLabel(voice.volumes[p.userId] ?? 100)}</strong></label>
            <input id="vol-{p.userId}" type="range" min="0" max="200" step="5" value={voice.volumes[p.userId] ?? 100}
              oninput={(e) => voiceApi.setVolume(p.userId, clampVolume(+e.currentTarget.value))} />
            <small>Only changes what you hear.</small>
            <button class="ghost" onclick={() => voiceApi.setVolume(p.userId, 100)}>Reset</button>
          </div>
        {/if}
      </div>
    {:else}
      <p class="empty">Nobody's here yet.</p>
    {/each}
  </div>

  <div class="bar-wrap">
    {#if inHere}
      <div class="controls">
        <button class="ctl" class:on={voice.controls.muted} aria-pressed={voice.controls.muted} aria-label={voice.controls.muted ? "Unmute" : "Mute"} onclick={() => voiceApi.toggleMute()}>
          <Icon name={voice.controls.muted ? 'micOff' : 'mic'} size={20} />
        </button>
        <button class="ctl" class:on={voice.controls.deafened} aria-pressed={voice.controls.deafened} aria-label={voice.controls.deafened ? "Undeafen" : "Deafen"} onclick={() => voiceApi.toggleDeafen()}>
          <Icon name={voice.controls.deafened ? 'headphonesOff' : 'headphones'} size={20} />
        </button>
        <button class="leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={22} /></button>
      </div>
    {:else}
      <button class="primary" onclick={() => voiceApi.join(channelId)}>Join voice</button>
    {/if}
  </div>
</div>

<style>
  .panel { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--ok); }
  .title { color: var(--text); font-size: 15px; font-weight: 600; flex: 1; }
  .status { font-size: 12px; font-weight: 600; color: var(--ok); padding: 4px 10px; border-radius: 12px; background: rgba(79, 209, 139, .12); }
  .status.warn { color: #e8b04a; background: rgba(232, 176, 74, .12); }
  .error { margin: 12px 18px 0; color: #f2616b; font-size: 13px; }
  .grid { flex: 1; padding: 20px; display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); grid-auto-rows: 200px; gap: 14px; overflow-y: auto; }
  .tile { position: relative; background: var(--bg-1); border-radius: 16px; border: 2px solid transparent; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; }
  .tile.speaking { border-color: var(--ok); }
  .avatar-btn { position: relative; border: 0; padding: 0; background: none; border-radius: 50%; }
  .avatar-btn:disabled { cursor: default; }
  .av { width: 84px; height: 84px; border-radius: 50%; color: #fff; font-size: 34px; font-weight: 700; display: grid; place-items: center; transition: box-shadow .12s; }
  .tile.speaking .av { box-shadow: 0 0 0 4px var(--bg-1), 0 0 0 7px var(--ok); }
  .badge { position: absolute; right: -4px; bottom: -4px; width: 28px; height: 28px; border-radius: 50%; background: var(--danger); color: #fff; border: 4px solid var(--bg-1); display: grid; place-items: center; }
  .name { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .bars { display: flex; align-items: flex-end; gap: 2px; }
  .bar { width: 3px; border-radius: 1px; background: var(--bg-4); }
  .bar.on { background: var(--ok); }
  .vol { position: absolute; top: 10px; right: 12px; font-size: 11px; color: var(--text-3); }
  .pop { position: absolute; top: calc(100% - 30px); left: 50%; transform: translateX(-50%); z-index: 5; width: 240px; padding: 14px; background: #2e3037; border: 1px solid #3d4048; border-radius: 14px; box-shadow: 0 18px 48px rgba(8, 9, 12, .5); display: flex; flex-direction: column; gap: 8px; font-size: 13px; }
  .pop label { display: flex; justify-content: space-between; color: var(--text-2); }
  .pop input { accent-color: var(--accent); }
  .pop small { color: var(--text-3); }
  .empty { color: var(--text-3); }
  .bar-wrap { height: 84px; flex-shrink: 0; display: grid; place-items: center; }
  .controls { padding: 8px; background: #17181c; border-radius: 18px; display: flex; gap: 8px; }
  .ctl { width: 48px; height: 48px; border: 0; border-radius: 14px; background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
  .ctl.on { background: var(--danger); color: #fff; }
  .leave { width: 64px; height: 48px; border: 0; border-radius: 14px; background: var(--danger); color: #fff; display: grid; place-items: center; }
  .primary { height: 44px; padding: 0 22px; border: 0; border-radius: 12px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .ghost { align-self: flex-start; height: 28px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
</style>
