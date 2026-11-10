<script lang="ts">
  // The call at the top of a DM or group (#50): who's in it (speaking ring, mute / deafen), and Join, or the usual
  // controls once you're in. While it rings you it says so, statically, with Join / Decline: no modal, no popover
  // here (the ringtone is the call store's). Right-click a person for their volume and the rest of the person menu.
  // Once you're in, Go Live and Soundboard are the voice panel's action row (#175, #221): the soundboard plays into
  // whatever call you're in, and each person's "Mute soundboard" (their right-click menu) holds here too.
  // Someone live here (#51): their LIVE badge (its popover has the preview) and Watch, as in a server voice channel;
  // the watch shows in full view in place of the conversation (the shell), or as the mini-preview once you go Back.
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import CallActions from './CallActions.svelte'
  import { app, openDm, ui } from '../lib/store.svelte'
  import { selfVoice, voice, voiceApi } from '../lib/voice.svelte'
  import { callRoster, displayName } from '../lib/selectors'
  import { menuPoint } from '../lib/participantMenu'
  import { stageLabel } from '../lib/voicestage'
  import { declineCall, joinCall, ringIn } from '../lib/calls.svelte'
  import { soundboard } from '../lib/soundboard.svelte'
  import LiveBadge from './LiveBadge.svelte'
  import { liveIn } from '../lib/selectors'
  import { isWindows } from '../lib/stream/platform'
  import { findWatch } from '../lib/stream/stream.svelte'
  import { watchDisabledReason } from '../lib/stream/viewer'
  import { openFull, watchModeFor, watchStream } from '../lib/stream/viewer.svelte'

  let { channelId, onVoiceSettings = () => {} }: { channelId: string; onVoiceSettings?: () => void } = $props()

  const call = $derived(app.state.calls[channelId])
  const ring = $derived(ringIn(channelId))
  const self = $derived(selfVoice())
  const inHere = $derived(self.channelId === channelId)
  const roster = $derived(callRoster(app.state, null, channelId, self))
  const shown = $derived(!!call || !!ring || inHere)
  const caller = $derived(ring ? displayName(app.state, null, ring.caller_id) : '')
  const label = $derived(
    ring && !inHere ? `${caller} is calling…` : inHere ? stageLabel(voice.stage) : roster.length ? 'Call in progress' : 'Calling…',
  )

  // who's live here, oldest stream first; your own shows its badge but no Watch
  const live = $derived(Object.values(liveIn(app.state, channelId)).sort((a, b) => a.started_at.localeCompare(b.started_at)))
  const windows = isWindows()
  const watchOff = $derived(windows ? null : watchDisabledReason({ windows, voiceChannelId: voice.channelId, channelId }))
  function watch(streamer: string) {
    if (watchOff) return
    // already watching: back to it in full view; out of the call, this joins first
    if (findWatch(channelId, streamer)) openFull({ channelId, streamer })
    else watchStream(channelId, streamer)
  }

  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent, anchor: HTMLElement) {
    e.preventDefault()
    menu = { userId, ...menuPoint(e, anchor), opener: anchor }
  }
  async function message(userId: string) {
    ui.goTo = await openDm(userId)
  }
</script>

{#if shown}
  <div class="band" class:ringing={!!ring && !inHere} role="region" aria-label="Call">
    <div class="people">
      {#each roster as p (p.userId)}
        <button type="button" class="who" class:speaking={voice.speaking.has(p.userId) || soundboard.playing.has(p.userId)} class:pending={p.pending}
          aria-label={p.name} data-tip={p.name} oncontextmenu={(e) => openMenu(p.userId, e, e.currentTarget)}>
          <Avatar id={p.userId} name={p.name} size={34} />
          {#if p.deafened}<span class="flag"><Icon name="headphonesOff" size={11} /></span>
          {:else if p.muted}<span class="flag"><Icon name="micOff" size={11} /></span>{/if}
        </button>
      {/each}
      {#if ring && !inHere && !roster.some((p) => p.userId === ring.caller_id)}
        <span class="who"><Avatar id={ring.caller_id} name={caller} size={34} /></span>
      {/if}
    </div>
    <span class="label"><Icon name="phone" size={14} /> {label}</span>
    {#each live as l (l.user_id)}
      {@const name = displayName(app.state, null, l.user_id)}
      <span class="live-who">
        <LiveBadge {channelId} userId={l.user_id} />
        <span class="live-name">{name}</span>
        {#if l.user_id !== app.state.me?.id}
          <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the reason -->
          <button type="button" class="watch" class:off={!!watchOff} aria-disabled={watchOff ? 'true' : undefined} data-tip={watchOff ?? undefined}
            aria-label="Watch {name}'s stream" onclick={() => watch(l.user_id)}>{findWatch(channelId, l.user_id) ? 'Open' : watchModeFor(channelId, l.user_id) === 'join' && !watchOff ? 'Join & watch' : 'Watch'}</button>
        {/if}
      </span>
    {/each}
    <span class="grow"></span>
    {#if inHere}
      <CallActions {channelId} />
      <button class="ctl" class:on={voice.controls.muted} aria-pressed={voice.controls.muted} aria-label={voice.controls.muted ? 'Unmute' : 'Mute'} onclick={() => voiceApi.toggleMute()}>
        <Icon name={voice.controls.muted ? 'micOff' : 'mic'} size={17} />
      </button>
      <button class="ctl" class:on={voice.controls.deafened} aria-pressed={voice.controls.deafened} aria-label={voice.controls.deafened ? 'Undeafen' : 'Deafen'} onclick={() => voiceApi.toggleDeafen()}>
        <Icon name={voice.controls.deafened ? 'headphonesOff' : 'headphones'} size={17} />
      </button>
      <button class="leave" aria-label="Leave call" onclick={() => voiceApi.leave()}><Icon name="hangup" size={18} /></button>
    {:else}
      <button class="join" onclick={() => joinCall(channelId)}><Icon name="phone" size={14} /> Join</button>
      {#if ring}<button class="decline" onclick={() => declineCall(channelId)}>Decline</button>{/if}
    {/if}
  </div>
{/if}

{#if menu}
  {#key menu}<ParticipantMenu {channelId} serverId={null} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} onMessage={message} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .band { flex-shrink: 0; display: flex; align-items: center; gap: 12px; padding: 10px 18px; background: var(--bg-inset); border-bottom: 1px solid var(--bg-3); }
  .band.ringing { background: var(--ok-soft); }
  .people { display: flex; gap: 6px; }
  .who { position: relative; padding: 0; border: 0; background: none; border-radius: 50%; }
  .who.pending { opacity: .5; }
  .who.speaking :global(.av) { box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--ok); }
  .flag { position: absolute; right: -3px; bottom: -3px; width: 17px; height: 17px; border-radius: 50%; display: grid; place-items: center; background: var(--danger); color: var(--on-danger); box-shadow: 0 0 0 2px var(--ring); }
  .label { display: flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 600; color: var(--text); }
  .band.ringing .label { color: var(--ok-text); }
  .grow { flex: 1; }
  .live-who { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text); }
  .live-name { font-weight: 600; }
  .watch { height: 28px; padding: 0 12px; border: 0; border-radius: var(--radius-md); background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .watch:hover { filter: brightness(1.1); }
  .watch.off { opacity: .5; cursor: default; filter: none; }
  .ctl { width: 36px; height: 36px; border: 0; border-radius: var(--radius-md); background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
  .ctl:hover { background: var(--bg-4); }
  .ctl.on { background: var(--danger); color: var(--on-danger); }
  .leave { width: 48px; height: 36px; border: 0; border-radius: var(--radius-md); background: var(--danger); color: var(--on-danger); display: grid; place-items: center; }
  .join { height: 34px; padding: 0 14px; border: 0; border-radius: var(--radius-md); background: var(--accent); color: var(--on-accent); font-weight: 600; display: flex; align-items: center; gap: 6px; }
  .decline { height: 34px; padding: 0 14px; border: 0; border-radius: var(--radius-md); background: var(--danger); color: var(--on-danger); font-weight: 600; }
  .join:hover, .decline:hover, .leave:hover, .ctl.on:hover { filter: brightness(1.1); }
</style>
