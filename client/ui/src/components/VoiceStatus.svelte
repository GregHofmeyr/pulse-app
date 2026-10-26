<script lang="ts">
  import Icon from './Icon.svelte'
  import LinkStatsPanel from './LinkStatsPanel.svelte'
  import { app } from '../lib/store.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { stageLabel } from '../lib/voicestage'

  const channelId = $derived(voice.channelId ?? voice.joining ?? voice.joinFailed?.channelId ?? null)
  const channelName = $derived((channelId && app.state.channels[channelId]?.name) || '')
  const visible = $derived(!!(voice.channelId || voice.joining || voice.joinFailed))
  const failed = $derived(voice.joinFailed && !voice.joining && !voice.channelId ? voice.joinFailed : null)
  const clickable = $derived(!failed && (voice.stage.stage === 'connected' || voice.stage.stage === 'starting_audio'))
  const warn = $derived(!clickable)
  let open = $state(false)

  // the panel only exists while there is a connection to describe
  $effect(() => {
    if (!clickable) open = false
  })
</script>

{#if visible}
  <div class="vc">
    {#if failed}
      <span class="vc-bars warn"><span></span><span></span><span></span></span>
      <span class="vc-text"><strong class="bad">Couldn't connect</strong><small title={failed.error}>{failed.error}</small></span>
      <button class="vc-retry" onclick={() => voiceApi.retry()}>Retry</button>
    {:else}
      {#if clickable}
        <button class="vc-main" aria-haspopup="dialog" aria-expanded={open} data-menu-toggle onclick={() => (open = !open)}>
          <span class="vc-bars"><span></span><span></span><span></span></span>
          <span class="vc-text"><strong>{stageLabel(voice.stage)}</strong><small>{channelName}</small></span>
        </button>
      {:else}
        <span class="vc-main">
          <span class="vc-bars warn"><span></span><span></span><span></span></span>
          <span class="vc-text"><strong class:warn>{stageLabel(voice.stage)}</strong><small>{channelName}</small></span>
        </span>
      {/if}
      <button class="vc-leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={17} /></button>
    {/if}
    {#if open && clickable}<LinkStatsPanel {channelName} onClose={() => (open = false)} />{/if}
  </div>
{/if}

<style>
  .vc { position: relative; margin: 0 8px 8px; padding: 10px 10px 10px 14px; background: #232429; border-radius: 12px; display: flex; align-items: center; gap: 10px; }
  .vc-main { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 0; border: 0; background: transparent; color: inherit; text-align: left; }
  button.vc-main { cursor: pointer; }
  .vc-bars { display: flex; align-items: flex-end; gap: 2px; height: 14px; }
  .vc-bars span { width: 3px; border-radius: 1px; background: var(--ok); }
  .vc-bars span:nth-child(1) { height: 5px; } .vc-bars span:nth-child(2) { height: 9px; } .vc-bars span:nth-child(3) { height: 14px; }
  .vc-bars.warn span { background: #e8b04a; }
  .vc-text { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .vc-text strong { font-size: 13px; color: var(--ok); }
  .vc-text strong.warn { color: #e8b04a; }
  .vc-text strong.bad { color: #f2616b; }
  .vc-text small { font-size: 12px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .vc-leave { width: 34px; height: 34px; border: 0; border-radius: 9px; background: #2e3037; color: #f2616b; display: grid; place-items: center; }
  .vc-retry { height: 30px; padding: 0 12px; border: 0; border-radius: 8px; background: var(--accent); color: var(--on-accent); font-size: 12px; font-weight: 600; }
</style>
