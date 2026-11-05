<script lang="ts">
  // The call at the top of a DM or group (#50): who's in it (speaking ring, mute / deafen), and Join, or the usual
  // controls once you're in. While it rings you it says so, statically, with Join / Decline: no modal, no popover
  // here (the ringtone is the call store's). Right-click a person for their volume and the rest of the person menu.
  // Once you're in, the soundboard is the voice panel's own popover and dialogs (#175): it plays into whatever call
  // you're in, and each person's "Mute soundboard" (their right-click menu) holds here too.
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import SoundboardPopover from './SoundboardPopover.svelte'
  import SoundDialog from './SoundDialog.svelte'
  import SoundTidy from './SoundTidy.svelte'
  import type { Sound } from '../lib/protocol/Sound'
  import { app, openDm, ui } from '../lib/store.svelte'
  import { selfVoice, voice, voiceApi } from '../lib/voice.svelte'
  import { callRoster, displayName } from '../lib/selectors'
  import { menuPoint } from '../lib/participantMenu'
  import { stageLabel } from '../lib/voicestage'
  import { declineCall, joinCall, ringIn } from '../lib/calls.svelte'
  import { soundboard } from '../lib/soundboard.svelte'

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

  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent, anchor: HTMLElement) {
    e.preventDefault()
    menu = { userId, ...menuPoint(e, anchor), opener: anchor }
  }
  async function message(userId: string) {
    ui.goTo = await openDm(userId)
  }

  let sbOpen = $state(false)
  let sbBtn = $state<HTMLButtonElement>()
  // as in the voice panel: the dialogs live outside the popover (its click-outside would close it under them)
  let sbDialog = $state<{ kind: 'add'; serverId: string } | { kind: 'edit'; sound: Sound } | { kind: 'tidy'; serverId: string } | null>(null)
  const openDialog = (d: NonNullable<typeof sbDialog>) => {
    sbOpen = false
    sbDialog = d
  }
  // leaving the call takes the button away, and the popover with it
  $effect(() => {
    if (!inHere) {
      sbOpen = false
      sbDialog = null
    }
  })
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
    <span class="grow"></span>
    {#if inHere}
      <button class="ctl" class:on={voice.controls.muted} aria-pressed={voice.controls.muted} aria-label={voice.controls.muted ? 'Unmute' : 'Mute'} onclick={() => voiceApi.toggleMute()}>
        <Icon name={voice.controls.muted ? 'micOff' : 'mic'} size={17} />
      </button>
      <button class="ctl" class:on={voice.controls.deafened} aria-pressed={voice.controls.deafened} aria-label={voice.controls.deafened ? 'Undeafen' : 'Deafen'} onclick={() => voiceApi.toggleDeafen()}>
        <Icon name={voice.controls.deafened ? 'headphonesOff' : 'headphones'} size={17} />
      </button>
      <button bind:this={sbBtn} class="ctl" class:open={sbOpen} aria-label="Soundboard" aria-haspopup="dialog" aria-expanded={sbOpen} data-tip="Soundboard" data-menu-toggle onclick={() => (sbOpen = !sbOpen)}>
        <Icon name="soundboard" size={17} />
      </button>
      <button class="leave" aria-label="Leave call" onclick={() => voiceApi.leave()}><Icon name="hangup" size={18} /></button>
    {:else}
      <button class="join" onclick={() => joinCall(channelId)}><Icon name="phone" size={14} /> Join</button>
      {#if ring}<button class="decline" onclick={() => declineCall(channelId)}>Decline</button>{/if}
    {/if}
  </div>
{/if}

{#if sbOpen && sbBtn}
  <SoundboardPopover
    anchor={sbBtn}
    onClose={() => (sbOpen = false)}
    onAdd={(serverId) => openDialog({ kind: 'add', serverId })}
    onEdit={(sound) => openDialog({ kind: 'edit', sound })}
    onTidy={(serverId) => openDialog({ kind: 'tidy', serverId })}
  />
{/if}

{#if sbDialog?.kind === 'tidy'}
  <SoundTidy serverId={sbDialog.serverId} onClose={() => (sbDialog = null)} />
{:else if sbDialog}
  <SoundDialog mode={sbDialog} onClose={() => (sbDialog = null)} />
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
  .ctl { width: 36px; height: 36px; border: 0; border-radius: var(--radius-md); background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
  .ctl:hover, .ctl.open { background: var(--bg-4); }
  .ctl.on { background: var(--danger); color: var(--on-danger); }
  .leave { width: 48px; height: 36px; border: 0; border-radius: var(--radius-md); background: var(--danger); color: var(--on-danger); display: grid; place-items: center; }
  .join { height: 34px; padding: 0 14px; border: 0; border-radius: var(--radius-md); background: var(--accent); color: var(--on-accent); font-weight: 600; display: flex; align-items: center; gap: 6px; }
  .decline { height: 34px; padding: 0 14px; border: 0; border-radius: var(--radius-md); background: var(--danger); color: var(--on-danger); font-weight: 600; }
  .join:hover, .decline:hover, .leave:hover, .ctl.on:hover { filter: brightness(1.1); }
</style>
