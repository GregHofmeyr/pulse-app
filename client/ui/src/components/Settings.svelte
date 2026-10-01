<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { loadAudioConfig, saveAudioConfig, type AudioConfig } from '../lib/voiceui'

  let { onClose }: { onClose: () => void } = $props()

  let cfg = $state<AudioConfig>(loadAudioConfig())
  let inputs = $state<{ name: string; is_default: boolean }[]>([])
  let outputs = $state<{ name: string; is_default: boolean }[]>([])
  let testing = $state(false)
  let error = $state('')
  const linux = navigator.userAgent.includes('Linux')

  onMount(async () => {
    try {
      const d = await voiceApi.listDevices()
      inputs = d.inputs
      outputs = d.outputs
    } catch {
      /* device list is optional */
    }
  })
  onDestroy(() => {
    if (testing) void voiceApi.stopMicTest()
  })

  async function apply() {
    saveAudioConfig(cfg)
    try {
      await voiceApi.setAudioConfig(cfg)
      if (testing) {
        await voiceApi.stopMicTest()
        await voiceApi.startMicTest(cfg)
      }
    } catch (e) {
      error = typeof e === 'string' ? e : 'Could not apply settings'
    }
  }

  async function toggleTest() {
    error = ''
    try {
      if (testing) await voiceApi.stopMicTest()
      else await voiceApi.startMicTest(cfg)
      testing = !testing
    } catch (e) {
      error = typeof e === 'string' ? e : 'Mic test failed'
    }
  }

  const toggles: { key: 'echo_cancel' | 'noise_suppress' | 'auto_gain'; label: string; help: string }[] = [
    { key: 'echo_cancel', label: 'Echo cancellation', help: 'Stops your speakers leaking back into your mic.' },
    { key: 'noise_suppress', label: 'Noise suppression', help: 'Removes fans, hum and background noise.' },
    { key: 'auto_gain', label: 'Automatic gain control', help: 'Evens out your volume. Leave off if you use input gain.' },
  ]
</script>

<div class="backdrop" role="presentation" onclick={onClose}></div>
<div class="modal" role="dialog" aria-label="Voice and audio settings">
  <div class="top">
    <h1>Voice &amp; Audio</h1>
    <button class="close" aria-label="Close settings" onclick={onClose}><Icon name="x" size={16} /></button>
  </div>

  <div class="two">
    <label>INPUT DEVICE
      <select bind:value={cfg.input} onchange={apply}>
        <option value={null}>System default</option>
        {#each inputs.filter((d) => d.name !== 'default') as d}<option value={d.name}>{d.name}</option>{/each}
      </select>
    </label>
    <label>OUTPUT DEVICE
      <select bind:value={cfg.output} onchange={apply}>
        <option value={null}>System default</option>
        {#each outputs.filter((d) => d.name !== 'default') as d}<option value={d.name}>{d.name}</option>{/each}
      </select>
    </label>
  </div>
  {#if linux}<p class="hint">On Linux, pick your mic and headphones in your system sound settings. Pulse follows the system default.</p>{/if}

  <div class="two">
    <label>INPUT GAIN <strong>{(cfg.input_gain_pct / 100).toFixed(1)}×</strong>
      <input type="range" min="50" max="400" step="10" bind:value={cfg.input_gain_pct} onchange={apply} />
      <small>For quiet mics: boosts your voice before it's sent.</small>
    </label>
    <label>INPUT SENSITIVITY
      <div class="meter" aria-hidden="true">
        <div class="fill" style:width="{Math.min(100, voice.levels.mic * 400)}%"></div>
        <div class="mark" style:left="{Math.min(100, cfg.sensitivity * 400)}%"></div>
      </div>
      <input type="range" min="0" max="0.2" step="0.002" bind:value={cfg.sensitivity} onchange={apply} aria-label="Sensitivity threshold" />
      <small>Only sound past the marker is sent. Talk to see the green bar move.</small>
    </label>
  </div>

  <div class="check">
    <button class="primary" onclick={toggleTest}>{testing ? 'Stop' : "Let's check"}</button>
    <small>{testing ? 'You should hear yourself after a moment. Headphones recommended.' : 'Hear your mic the way others will.'}</small>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="toggles">
    {#each toggles as t}
      <div class="row">
        <div><strong>{t.label}</strong><small>{t.help}</small></div>
        <button class="switch" aria-label={t.label} aria-pressed={cfg[t.key]} class:on={cfg[t.key]}
          onclick={() => { cfg[t.key] = !cfg[t.key]; apply() }}><span></span></button>
      </div>
    {/each}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(10, 11, 13, .6); z-index: 20; }
  .modal { position: fixed; z-index: 21; top: 50%; left: 50%; transform: translate(-50%, -50%); width: min(760px, calc(100vw - 32px)); max-height: calc(100vh - 64px); overflow-y: auto; padding: 28px 32px; background: var(--bg-1); border-radius: 16px; border: 1px solid var(--bg-3); display: flex; flex-direction: column; gap: 20px; }
  .top { display: flex; justify-content: space-between; align-items: center; }
  h1 { margin: 0; font-size: 22px; font-weight: 600; }
  .close { width: 36px; height: 36px; border: 1px solid var(--bg-4); border-radius: 50%; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .two { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 20px; }
  label { display: flex; flex-direction: column; gap: 8px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  label strong { color: var(--text); letter-spacing: 0; }
  select { height: 42px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 10px; background: #26282e; font-size: 14px; letter-spacing: 0; }
  input[type='range'] { accent-color: var(--accent); }
  small { font-size: 12px; font-weight: 400; color: var(--text-3); letter-spacing: 0; }
  .hint { margin: -8px 0 0; font-size: 12px; color: var(--text-3); }
  .meter { position: relative; height: 12px; border-radius: 6px; background: #2f3138; overflow: hidden; }
  .fill { height: 100%; background: var(--ok); transition: width .1s linear; }
  .mark { position: absolute; top: 0; width: 3px; height: 100%; background: #f6f7f9; }
  .check { display: flex; align-items: center; gap: 12px; padding: 16px; background: var(--bg-2); border-radius: 14px; }
  .primary { height: 38px; padding: 0 16px; border: 0; border-radius: 10px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .error { margin: 0; color: #f2616b; font-size: 13px; }
  .toggles { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 16px; padding: 12px 0; border-bottom: 1px solid #26282e; }
  .row > div { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .switch { position: relative; width: 44px; height: 24px; flex-shrink: 0; border: 0; border-radius: 12px; padding: 0; background: #3a3c44; }
  .switch span { position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: #b9bcc6; transition: left .12s; }
  .switch.on { background: var(--accent); }
  .switch.on span { left: 23px; background: var(--on-accent); }
</style>
