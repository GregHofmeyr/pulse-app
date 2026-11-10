<script lang="ts">
  // A voice channel's settings (#75), from its right-click menu: anyone in the server may change them (the v1 trust
  // model, as for renaming). For now its audio bitrate: everyone publishing in the channel sends at it, and a call in
  // progress switches at once. A pick saves straight away, like Settings; everyone hears ChannelUpdated.
  import { untrack } from 'svelte'
  import { modal } from '../lib/actions'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { bitrateChoices, fromChoice, toChoice } from '../lib/channelBitrate'
  import Select from './Select.svelte'
  import type { Channel } from '../lib/protocol/Channel'

  let { channel, onClose }: { channel: Channel; onClose: () => void } = $props()

  // the channel as everyone has it now (someone else may change it while this is open)
  const live = $derived(app.state.channels[channel.id] ?? channel)
  let value = $state(untrack(() => toChoice(channel.bitrate)))
  let error = $state('')
  let saving = $state(false)
  // another person's change lands here too, unless a pick of ours is on its way
  $effect(() => {
    const now = toChoice(live.bitrate)
    if (!untrack(() => saving)) value = now
  })

  async function save() {
    error = ''
    saving = true
    try {
      await api.setChannelBitrate(channel.id, fromChoice(value))
    } catch (e) {
      error = errorText(e)
      value = toChoice(live.bitrate)
    } finally {
      saving = false
    }
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Channel settings" tabindex="-1" use:modal={onClose}>
    <h1 class="modal-title">Channel settings</h1>
    <p class="name">{live.name}</p>
    <div class="settings-field"><span class="settings-label">BITRATE</span>
      <Select label="Bitrate" bind:value options={bitrateChoices} onchange={save} />
    </div>
    <p class="hint">Higher sounds better, music especially, but uses more data for everyone in the call.</p>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-primary" onclick={onClose}>Done</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  /* small enough never to scroll, so the bitrate list can open past its bottom edge */
  .modal { width: min(400px, 100%); overflow: visible; }
  p { margin: 0; overflow-wrap: anywhere; }
  .name { margin-top: -8px; font-size: 14px; color: var(--text-2); }
  /* as in Voice & Audio */
  .hint { margin: -4px 0 0; font-size: 12px; color: var(--text-3); }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
