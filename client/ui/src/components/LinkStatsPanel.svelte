<script lang="ts">
  import { clickOutside } from '../lib/actions'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { detailsText, jitterLevel, lossLevel, pingLevel, sparklinePath, transportLabel } from '../lib/voicelink'

  let { channelName, onClose }: { channelName: string; onClose: () => void } = $props()

  const link = $derived(voice.link)
  let copied = $state(false)

  // Polling runs only while the panel exists and the window is focused; the cleanup stops it on blur and on close.
  $effect(() => {
    if (!windowFocus.focused) return
    void voiceApi.watchLink(true)
    return () => void voiceApi.watchLink(false)
  })

  const W = 220
  const H = 36
  const path = $derived(link ? sparklinePath(link.history, W, H) : '')

  async function copy() {
    if (!link) return
    try {
      await navigator.clipboard.writeText(detailsText(link, channelName))
      copied = true
      setTimeout(() => (copied = false), 2000)
    } catch {
      /* clipboard unavailable */
    }
  }
</script>

<div class="panel" role="dialog" aria-label="Connection details" use:clickOutside={onClose}>
  <dl>
    <dt>Ping</dt>
    <dd class="ping {link?.rtt_ms != null ? pingLevel(link.rtt_ms) : ''}">{link?.rtt_ms != null ? `${Math.round(link.rtt_ms)} ms` : '—'}</dd>
    <dt>Jitter</dt>
    <dd class="jitter {link?.jitter_ms != null ? jitterLevel(link.jitter_ms) : ''}">{link?.jitter_ms != null ? `${Math.round(link.jitter_ms)} ms` : '—'}</dd>
    <dt>Packet loss</dt>
    <dd class="loss {link?.loss_pct != null ? lossLevel(link.loss_pct) : ''}">{link?.loss_pct != null ? `${Math.round(link.loss_pct * 10) / 10}%` : '—'}</dd>
  </dl>
  <svg class="spark" viewBox="0 0 {W} {H}" role="img" aria-label="Ping over the last 60 seconds" preserveAspectRatio="none">
    {#if path}<path d={path} fill="none" stroke="currentColor" stroke-width="1.5" vector-effect="non-scaling-stroke" />{/if}
  </svg>
  <small class="span">Ping, last 60 s</small>
  <dl class="meta">
    <dt>Voice server</dt>
    <dd>{link?.server ?? '—'}</dd>
    <dt>Connection type</dt>
    <dd>{link?.transport ? transportLabel(link.transport) : '—'}</dd>
  </dl>
  <button class="copy" disabled={!link} onclick={copy}>{copied ? 'Copied' : 'Copy details'}</button>
</div>

<style>
  .panel { position: absolute; left: 0; right: 0; bottom: calc(100% + 6px); z-index: 15; padding: 14px; background: var(--bg-1); border: 1px solid var(--bg-4); border-radius: 12px; display: flex; flex-direction: column; gap: 8px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35); }
  dl { margin: 0; display: grid; grid-template-columns: 1fr auto; gap: 4px 12px; font-size: 13px; }
  dt { color: var(--text-3); }
  dd { margin: 0; font-weight: 600; text-align: right; overflow-wrap: anywhere; }
  .good { color: var(--ok); }
  .warn { color: var(--warn); }
  .bad { color: var(--danger); }
  .spark { width: 100%; height: 36px; color: var(--accent); background: var(--bg-2); border-radius: 6px; }
  .span { color: var(--text-3); font-size: 11px; margin-top: -4px; }
  .copy { height: 30px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
  .copy:disabled { opacity: .5; }
</style>
