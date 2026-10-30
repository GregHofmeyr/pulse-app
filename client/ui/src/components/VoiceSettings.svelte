<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Select from './Select.svelte'
  import Segmented from './Segmented.svelte'
  import SoundSettings from './SoundSettings.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { loadAudioConfig, saveAudioConfig, type AudioConfig, type NsLevel } from '../lib/voiceui'

  let cfg = $state<AudioConfig>(loadAudioConfig())
  let inputs = $state<{ name: string; is_default: boolean }[]>([])
  let outputs = $state<{ name: string; is_default: boolean }[]>([])
  let testing = $state(false)
  const SENS_MAX = 0.2
  // how long the mic has been completely silent (drives the "no sound" hint)
  let lastSound = Date.now()
  let silentFor = $state(0)
  $effect(() => {
    if (voice.levels.mic > 0.0005) lastSound = Date.now()
    silentFor = Date.now() - lastSound
  })
  $effect(() => {
    if (testing) lastSound = Date.now()
  })
  let error = $state('')
  const linux = navigator.userAgent.includes('Linux')
  // Linux lists real devices through the sound server; only ALSA's fallback has nothing but "default"
  let listed = $state(false)
  const noSoundServer = $derived(linux && listed && [...inputs, ...outputs].every((d) => d.name === 'default'))

  // The in-call meter: the core only sends levels while this page is open.
  onMount(() => {
    void voiceApi.watchMeters(true).catch(() => {})
    return () => void voiceApi.watchMeters(false).catch(() => {})
  })
  onMount(async () => {
    try {
      const d = await voiceApi.listDevices()
      inputs = d.inputs
      outputs = d.outputs
    } catch {
      /* device list is optional */
    }
    listed = true
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

  const toggles: { key: 'echo_cancel' | 'auto_gain'; label: string; help: string }[] = [
    { key: 'echo_cancel', label: 'Echo cancellation', help: 'Stops your speakers leaking back into your mic.' },
    { key: 'auto_gain', label: 'Automatic gain control', help: 'Evens out your volume. Leave off if you use input gain.' },
  ]
  const levels: { value: NsLevel; label: string; help: string }[] = [
    { value: 'off', label: 'Off', help: 'No noise suppression: your mic as it is.' },
    { value: 'standard', label: 'Standard', help: 'Light: removes fans and hum, softens clicks. Easy on older PCs.' },
    { value: 'strong', label: 'Strong', help: 'Removes keyboard clicks and most background noise. Uses more CPU.' },
  ]
</script>

<section class="settings-pane">
<h2>Voice &amp; Audio</h2>
  <div class="two">
  <div class="settings-field"><span class="settings-label">INPUT DEVICE</span>
    <Select label="Input device" bind:value={cfg.input} onchange={apply}
      options={[{ value: null, label: 'System default' }, ...inputs.filter((d) => d.name !== 'default').map((d) => ({ value: d.name, label: d.name }))]} />
  </div>
  <div class="settings-field"><span class="settings-label">OUTPUT DEVICE</span>
    <Select label="Output device" bind:value={cfg.output} onchange={apply}
      options={[{ value: null, label: 'System default' }, ...outputs.filter((d) => d.name !== 'default').map((d) => ({ value: d.name, label: d.name }))]} />
  </div>
</div>
{#if noSoundServer}<p class="hint">No sound server found, so pick your mic and headphones in your system sound settings. Pulse follows the system default.</p>{/if}

<div class="two">
  <label>INPUT GAIN <strong>{(cfg.input_gain_pct / 100).toFixed(1)}×</strong>
    <input type="range" min="50" max="400" step="10" bind:value={cfg.input_gain_pct} onchange={apply} />
    <small>For quiet mics: boosts your voice before it's sent.</small>
  </label>
  <div class="settings-field"><span class="settings-label">INPUT SENSITIVITY</span>
    <!-- the slider sits ON the level meter, so its thumb is the threshold marker -->
    <div class="row auto">
      <div><strong>Automatically determine sensitivity</strong></div>
      <button class="switch" aria-label="Automatically determine sensitivity" aria-pressed={cfg.auto_sensitivity} class:on={cfg.auto_sensitivity}
        onclick={() => { cfg.auto_sensitivity = !cfg.auto_sensitivity; apply() }}><span></span></button>
    </div>
    <!-- the slider sits ON the level meter, so its thumb is the threshold marker -->
    <div class="sens">
      <div class="meter" aria-hidden="true"><div class="fill" class:over={cfg.auto_sensitivity ? voice.gateOpen : voice.levels.mic >= cfg.sensitivity} style:width="{Math.min(100, (voice.levels.mic / SENS_MAX) * 100)}%"></div></div>
      {#if !cfg.auto_sensitivity}
        <input type="range" min="0" max={SENS_MAX} step="0.002" bind:value={cfg.sensitivity} onchange={apply} aria-label="Sensitivity threshold" />
      {/if}
    </div>
    <small>{cfg.auto_sensitivity ? 'Pulse sends your voice when it hears you speaking.' : 'Sound to the right of the handle is sent.'} {testing || voice.channelId ? 'Talk to see the bar move.' : "Start “Let's check” to see your level."}</small>
  </div>
</div>

<div class="settings-field"><span class="settings-label">NOISE SUPPRESSION</span>
  <Segmented label="Noise suppression" options={levels} value={cfg.noise_suppression} onchange={(v) => { cfg.noise_suppression = v; apply() }} />
  <small>{levels.find((l) => l.value === cfg.noise_suppression)?.help}</small>
  {#if voice.ns.note}<p class="warn" role="status">{voice.ns.note}</p>{/if}
</div>

<div class="check">
  <button class="primary" onclick={toggleTest}>{testing ? 'Stop' : "Let's check"}</button>
  <small>{testing ? 'You should hear yourself after a moment. Headphones recommended.' : 'Hear your mic the way others will.'}</small>
</div>
{#if testing && silentFor > 3000}
  <p class="warn" role="status">No sound from your mic. Check it isn't muted, or pick a different input in your system sound settings. Some Bluetooth headsets need a different headset codec.</p>
{/if}
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

<SoundSettings />
</section>

<style>
  .two { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 20px; }
  label { display: flex; flex-direction: column; gap: 8px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  label strong { color: var(--text); letter-spacing: 0; }
  input[type='range'] { accent-color: var(--accent); }
  .hint { margin: -8px 0 0; font-size: 12px; color: var(--text-3); }
  .sens { position: relative; height: 22px; display: flex; align-items: center; }
  .meter { position: absolute; left: 0; right: 0; height: 12px; border-radius: 6px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; background: #3a7d5a; transition: width .1s linear; }
  .fill.over { background: var(--ok); }
  .sens input[type='range'] { position: relative; width: 100%; margin: 0; background: transparent; -webkit-appearance: none; appearance: none; height: 22px; }
  .sens input[type='range']::-webkit-slider-runnable-track { background: transparent; height: 22px; }
  .sens input[type='range']::-webkit-slider-thumb { -webkit-appearance: none; width: 6px; height: 22px; border-radius: 3px; background: var(--text); box-shadow: 0 0 0 2px var(--bg-1); }
  .warn { margin: -8px 0 0; padding: 10px 12px; border-radius: 10px; background: var(--warn-soft); color: var(--warn); font-size: 13px; }
  .check { display: flex; align-items: center; gap: 12px; padding: 16px; background: var(--bg-2); border-radius: 14px; }
  .primary { height: 38px; padding: 0 16px; border: 0; border-radius: 10px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .primary:disabled { opacity: .5; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .toggles { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 16px; padding: 12px 0; border-bottom: 1px solid var(--line-soft); }
  .row > div { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .row.auto { padding: 0; border: 0; }
</style>
