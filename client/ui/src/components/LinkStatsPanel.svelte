<script lang="ts">
  import { onMount } from 'svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { detailsText, jitterLevel, lossLevel, pingLevel, reading, sparklinePath, transportLabel } from '../lib/voicelink'

  let { channelName, onClose, triggerSelector }: { channelName: string; onClose: (refocus: boolean) => void; triggerSelector: string } = $props()

  const link = $derived(voice.link)
  let copied = $state(false)
  let panel: HTMLElement

  const ping = $derived(reading(link?.rtt_ms ?? null, 0, pingLevel, ' ms'))
  const jitter = $derived(reading(link?.jitter_ms ?? null, 0, jitterLevel, ' ms'))
  const loss = $derived(reading(link?.loss_pct ?? null, 1, lossLevel, '%'))

  // a dialog takes focus when it opens; a press outside closes it without stealing focus
  onMount(() => {
    panel.focus()
    const down = (e: PointerEvent) => {
      const t = e.target as Element
      if (!panel.contains(t) && !t.closest?.(triggerSelector)) onClose(false)
    }
    window.addEventListener('pointerdown', down, true)
    return () => window.removeEventListener('pointerdown', down, true)
  })

  // Escape is handled where the focus is, so it never closes the panel for a key meant for another widget
  function onkeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    e.preventDefault()
    onClose(true)
  }

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

<div class="panel menu-surface" role="dialog" aria-label="Connection details" tabindex="-1" bind:this={panel} {onkeydown}>
  <dl>
    <dt>Ping</dt>
    <dd class="ping {ping.level}">{ping.text}</dd>
    <dt>Jitter</dt>
    <dd class="jitter {jitter.level}">{jitter.text}</dd>
    <dt>Packet loss</dt>
    <dd class="loss {loss.level}">{loss.text}</dd>
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
  <button class="btn btn-secondary copy" disabled={!link} onclick={copy}>{copied ? 'Copied' : 'Copy details'}</button>
</div>

<style>
  .panel { outline: none; position: absolute; left: 0; right: 0; bottom: calc(100% + 6px); z-index: 15; padding: 14px; gap: 8px; }
  dl { margin: 0; display: grid; grid-template-columns: 1fr auto; gap: 4px 12px; font-size: 13px; }
  dt { color: var(--text-3); }
  dd { margin: 0; font-weight: 600; text-align: right; overflow-wrap: anywhere; }
  .good { color: var(--ok); }
  .warn { color: var(--warn); }
  .bad { color: var(--danger-text); }
  .spark { width: 100%; height: 36px; color: var(--accent); background: var(--bg-2); border-radius: 6px; }
  .span { color: var(--text-3); font-size: 11px; margin-top: -4px; }
</style>
