<script lang="ts">
  import Icon from './Icon.svelte'
  import LinkStatsPanel from './LinkStatsPanel.svelte'
  import { app } from '../lib/store.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { stageLabel } from '../lib/voicestage'
  import { isWindows } from '../lib/stream/platform'

  // during a switch, the channel being joined
  const channelId = $derived(voice.joining ?? voice.channelId ?? voice.joinFailed?.channelId ?? null)
  const channelName = $derived((channelId && app.state.channels[channelId]?.name) || '')
  const visible = $derived(!!(voice.channelId || voice.joining || voice.joinFailed))
  const failed = $derived(voice.joinFailed && !voice.joining && !voice.channelId ? voice.joinFailed : null)
  const clickable = $derived(!failed && (voice.stage.stage === 'connected' || voice.stage.stage === 'starting_audio'))
  const warn = $derived(!clickable)
  let open = $state(false)
  let trigger = $state<HTMLButtonElement>()
  const TRIGGER = '[data-voice-status-trigger]'

  function close(refocus: boolean) {
    open = false
    if (refocus) trigger?.focus()
  }

  let box = $state<HTMLDivElement>()
  let status = $state<HTMLSpanElement>()
  let refocus = false
  // before the trigger and panel go: was focus on them? (afterwards it has already dropped to the body)
  // Focus elsewhere in the block (Leave) stays where it is.
  $effect.pre(() => {
    if (clickable) return
    const at = document.activeElement
    if (at && (trigger?.contains(at) || box?.querySelector('[role=dialog]')?.contains(at))) refocus = true
  })
  // the panel only exists while there is a connection to describe
  $effect(() => {
    if (clickable) return
    open = false
    if (refocus && status) status.focus()
    refocus = false
  })
</script>

{#if visible}
  <div class="vc" bind:this={box}>
    {#if failed}
      <span class="vc-bars warn"><span></span><span></span><span></span></span>
      <span class="vc-text"><strong class="bad">Couldn't connect</strong><small title={failed.error}>{failed.error}</small></span>
      <button class="vc-retry" onclick={() => voiceApi.retry()}>Retry</button>
      <button class="vc-dismiss" aria-label="Dismiss" onclick={() => voiceApi.dismissFailure()}><Icon name="x" size={14} /></button>
    {:else}
      {#if clickable}
        <button class="vc-main" aria-haspopup="dialog" aria-expanded={open} data-voice-status-trigger bind:this={trigger} onkeydown={(e) => { if (open && e.key === 'Escape' && !e.defaultPrevented) { e.preventDefault(); close(true) } }} onclick={() => (open = !open)}>
          <span class="vc-bars"><span></span><span></span><span></span></span>
          <span class="vc-text"><strong>{stageLabel(voice.stage)}</strong><small>{channelName}</small></span>
        </button>
      {:else}
        <span class="vc-main" tabindex="-1" bind:this={status}>
          <span class="vc-bars warn"><span></span><span></span><span></span></span>
          <span class="vc-text"><strong class:warn>{stageLabel(voice.stage)}</strong><small>{channelName}</small></span>
        </span>
      {/if}
      <button class="vc-leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={17} /></button>
    {/if}
    {#if open && clickable}<LinkStatsPanel {channelName} onClose={close} triggerSelector={TRIGGER} />{/if}
  </div>
  {#if voice.micSilent && voice.channelId}
    <p class="vc-mic" role="status">
      <strong>No sound from your mic.</strong>
      Check it isn't muted on the headset{isWindows() ? ", and that Windows lets apps use the microphone (Settings → Privacy → Microphone)" : ''}.
    </p>
  {/if}
{/if}

<style>
  .vc { position: relative; margin: 0 8px 8px; padding: 10px 10px 10px 14px; background: var(--bg-2); border-radius: 12px; display: flex; align-items: center; gap: 10px; }
  .vc-main { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 0; border: 0; background: transparent; color: inherit; text-align: left; }
  button.vc-main { cursor: pointer; }
  .vc-bars { display: flex; align-items: flex-end; gap: 2px; height: 14px; }
  .vc-bars span { width: 3px; border-radius: 1px; background: var(--ok); }
  .vc-bars span:nth-child(1) { height: 5px; } .vc-bars span:nth-child(2) { height: 9px; } .vc-bars span:nth-child(3) { height: 14px; }
  .vc-bars.warn span { background: var(--warn); }
  .vc-text { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .vc-text strong { font-size: 13px; color: var(--ok); }
  .vc-text strong.warn { color: var(--warn); }
  .vc-text strong.bad { color: var(--danger); }
  .vc-text small { font-size: 12px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .vc-leave { width: 34px; height: 34px; border: 0; border-radius: 9px; background: var(--surface-menu); color: var(--danger); display: grid; place-items: center; }
  .vc-dismiss { width: 28px; height: 28px; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .vc-mic { margin: -4px 8px 8px; padding: 8px 12px; border-radius: 10px; background: var(--bg-2); border-left: 3px solid var(--warn); font-size: 12px; line-height: 1.4; color: var(--text-2); }
  .vc-mic strong { color: var(--warn); }
  .vc-retry { height: 30px; padding: 0 12px; border: 0; border-radius: 8px; background: var(--accent); color: var(--on-accent); font-size: 12px; font-weight: 600; }
</style>
