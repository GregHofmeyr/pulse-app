<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import Select from './Select.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { loadAudioConfig, saveAudioConfig, type AudioConfig, type NsLevel } from '../lib/voiceui'

  let { onClose }: { onClose: () => void } = $props()

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

<div class="overlay">
<div class="backdrop" role="presentation" onclick={onClose}></div>
<div class="modal" role="dialog" aria-label="Voice and audio settings">
  <div class="top">
    <h1>Voice &amp; Audio</h1>
    <button class="close" aria-label="Close settings" onclick={onClose}><Icon name="x" size={16} /></button>
  </div>

  <div class="two">
    <div class="field"><span class="lbl">INPUT DEVICE</span>
      <Select label="Input device" bind:value={cfg.input} onchange={apply}
        options={[{ value: null, label: 'System default' }, ...inputs.filter((d) => d.name !== 'default').map((d) => ({ value: d.name, label: d.name }))]} />
    </div>
    <div class="field"><span class="lbl">OUTPUT DEVICE</span>
      <Select label="Output device" bind:value={cfg.output} onchange={apply}
        options={[{ value: null, label: 'System default' }, ...outputs.filter((d) => d.name !== 'default').map((d) => ({ value: d.name, label: d.name }))]} />
    </div>
  </div>
  {#if linux}<p class="hint">On Linux, pick your mic and headphones in your system sound settings. Pulse follows the system default.</p>{/if}

  <div class="two">
    <label>INPUT GAIN <strong>{(cfg.input_gain_pct / 100).toFixed(1)}×</strong>
      <input type="range" min="50" max="400" step="10" bind:value={cfg.input_gain_pct} onchange={apply} />
      <small>For quiet mics: boosts your voice before it's sent.</small>
    </label>
    <div class="field"><span class="lbl">INPUT SENSITIVITY</span>
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

  <div class="field"><span class="lbl">NOISE SUPPRESSION</span>
    <div class="seg" role="radiogroup" aria-label="Noise suppression">
      {#each levels as l}
        <button role="radio" aria-checked={cfg.noise_suppression === l.value} class:on={cfg.noise_suppression === l.value}
          onclick={() => { cfg.noise_suppression = l.value; apply() }}>{l.label}</button>
      {/each}
    </div>
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
</div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs text on sub-pixel transforms */
  .overlay { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 32px 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(760px, calc(100vw - 32px)); max-height: calc(100vh - 64px); overflow-y: auto; padding: 28px 32px; background: var(--bg-1); border-radius: 16px; border: 1px solid var(--bg-3); display: flex; flex-direction: column; gap: 20px; }
  .top { display: flex; justify-content: space-between; align-items: center; }
  h1 { margin: 0; font-size: 22px; font-weight: 600; }
  .close { width: 36px; height: 36px; border: 1px solid var(--bg-4); border-radius: 50%; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .two { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 20px; }
  label { display: flex; flex-direction: column; gap: 8px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  label strong { color: var(--text); letter-spacing: 0; }
  input[type='range'] { accent-color: var(--accent); }
  small { font-size: 12px; font-weight: 400; color: var(--text-3); letter-spacing: 0; }
  .hint { margin: -8px 0 0; font-size: 12px; color: var(--text-3); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .lbl { font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  .sens { position: relative; height: 22px; display: flex; align-items: center; }
  .meter { position: absolute; left: 0; right: 0; height: 12px; border-radius: 6px; background: #2f3138; overflow: hidden; }
  .fill { height: 100%; background: #3a7d5a; transition: width .1s linear; }
  .fill.over { background: var(--ok); }
  .sens input[type='range'] { position: relative; width: 100%; margin: 0; background: transparent; -webkit-appearance: none; appearance: none; height: 22px; }
  .sens input[type='range']::-webkit-slider-runnable-track { background: transparent; height: 22px; }
  .sens input[type='range']::-webkit-slider-thumb { -webkit-appearance: none; width: 6px; height: 22px; border-radius: 3px; background: #f6f7f9; box-shadow: 0 0 0 2px var(--bg-1); }
  .warn { margin: -8px 0 0; padding: 10px 12px; border-radius: 10px; background: rgba(232, 176, 74, .12); color: #e8b04a; font-size: 13px; }
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
  .row.auto { padding: 0; border: 0; }
  .seg { display: inline-flex; align-self: flex-start; border: 1px solid var(--bg-4); border-radius: 10px; overflow: hidden; }
  .seg button { padding: 8px 14px; border: 0; background: transparent; color: var(--text-2); font-weight: 600; }
  .seg button.on { background: var(--accent); color: var(--on-accent); }
</style>
