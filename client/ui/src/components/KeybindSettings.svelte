<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { Recorder, actions, bindLabel, keybindsApi, type Action, type Keybinds, type Recorded } from '../lib/keybinds'

  const linux = navigator.userAgent.includes('Linux')

  let binds = $state<Keybinds | null>(null)
  /** The master switch: off, the bind editors are disabled and (Windows) no hooks run. */
  let enabled = $state(true)
  let recording = $state<Action | null>(null)
  let held = $state('')
  let error = $state('')
  let problems = $state<string[]>([])
  let recorder = new Recorder()
  /** The mouse button just recorded: its mouseup/auxclick is swallowed too, or the side buttons
   *  would navigate the webview back/forward after recording ends. */
  let swallowButton: number | null = null

  onMount(async () => {
    try {
      const r = await keybindsApi.get()
      binds = r.binds
      enabled = r.enabled
      problems = r.problems
    } catch (e) {
      error = typeof e === 'string' ? e : "Couldn't load keybinds"
    }
  })

  async function save(next: Keybinds) {
    try {
      const r = await keybindsApi.set(next)
      binds = r.binds
      problems = r.problems
      error = ''
    } catch (e) {
      error = typeof e === 'string' ? e : "Couldn't save keybinds"
    }
  }

  async function setEnabled(on: boolean) {
    try {
      const r = await keybindsApi.setEnabled(on)
      binds = r.binds
      enabled = r.enabled
      problems = r.problems
      error = ''
    } catch (e) {
      error = typeof e === 'string' ? e : "Couldn't switch keybinds"
    }
  }

  async function record(action: Action) {
    if (!enabled) return
    if (recording === action) return stop()
    error = ''
    held = ''
    recorder = new Recorder()
    recording = action
    // the key being recorded mustn't also toggle mute/deafen
    await keybindsApi.pause(true).catch(() => {})
  }

  async function stop() {
    recording = null
    held = ''
    await keybindsApi.pause(false).catch(() => {})
  }

  async function finish(r: Recorded) {
    if (!recording || !binds || r.kind === 'none') return
    if (r.kind === 'held') return void (held = r.label)
    // stop first: a second keydown (or auto-repeat) during the save mustn't save again
    const action = recording
    recording = null
    held = ''
    if (r.kind === 'bind') await save({ ...binds, [action]: r.bind })
    await keybindsApi.pause(false).catch(() => {})
  }

  function swallow(e: Event) {
    e.preventDefault()
    e.stopPropagation()
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!recording) return
    swallow(e)
    void finish(recorder.down(e))
  }

  function onKeyUp(e: KeyboardEvent) {
    if (!recording) return
    swallow(e)
    void finish(recorder.up(e))
  }

  // wheel click and the side buttons; left/right clicks keep their normal meaning
  function onMouseDown(e: MouseEvent) {
    if (!recording || e.button === 0 || e.button === 2) return
    swallow(e)
    const r = recorder.mouse(e)
    if (r.kind === 'bind') swallowButton = e.button
    void finish(r)
  }

  function onMouseAfter(e: MouseEvent) {
    if (e.button !== swallowButton && !(recording && e.button !== 0 && e.button !== 2)) return
    swallow(e)
    if (e.type === 'auxclick') swallowButton = null
  }

  onDestroy(() => {
    if (recording) void keybindsApi.pause(false).catch(() => {})
  })
</script>

<!-- leaving the window mid-recording ends it, or the hotkeys would stay released -->
<svelte:window onkeydowncapture={onKeyDown} onkeyupcapture={onKeyUp} onmousedowncapture={onMouseDown}
  onmouseupcapture={onMouseAfter} onauxclickcapture={onMouseAfter} onblur={() => recording && stop()} />

<section class="pane">
  <h2>Keybinds</h2>
  {#if binds}
    <div class="row master">
      <div>
        <strong>Global keybinds</strong>
        {#if linux}
          <small>Saved for Windows. On Linux Pulse never watches the keyboard or mouse itself: your desktop runs the commands below.</small>
        {:else}
          <small>Pulse watches the keyboard and mouse system-wide only to match these binds; nothing is recorded or sent. Turning this off removes the hooks entirely.</small>
        {/if}
      </div>
      <button class="switch" aria-label="Global keybinds" aria-pressed={enabled} class:on={enabled} disabled={!!recording}
        onclick={() => setEnabled(!enabled)}><span></span></button>
    </div>
  {/if}
  {#if linux}
    <p class="hint">On Linux, apps can't grab keys for themselves: bind these commands in your desktop's keyboard settings (Hyprland: <code>bindn = , Next, exec, pulse-app --toggle-mute</code>; <code>bindn</code> keeps the key working in other apps).</p>
    <div class="list">
      {#each actions as a}
        <div class="row"><strong>{a.label}</strong><code>pulse-app --{a.key.replace('_', '-')}</code></div>
      {/each}
    </div>
  {:else if binds}
    <p class="hint">Works everywhere and the key keeps working in other apps. Doesn't work while an app running as administrator is focused.</p>
    <div class="list">
      {#each actions as a}
        <div class="row">
          <strong>{a.label}</strong>
          <button class="key" class:recording={recording === a.key} aria-label="Change {a.label}" disabled={!enabled}
            onclick={() => record(a.key)}>
            {recording === a.key ? held || 'Press keys…' : bindLabel(binds[a.key])}
          </button>
          <button class="clear" aria-label="Remove {a.label} bind" title="Remove" disabled={!enabled || !binds[a.key] || !!recording}
            onclick={() => binds && save({ ...binds, [a.key]: null })}><Icon name="x" size={14} /></button>
        </div>
      {/each}
    </div>
    {#if recording}<small>Press a key, a combination or a mouse button. Esc cancels.</small>{/if}
    {#each problems as p}<p class="warn" role="status">{p}</p>{/each}
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

<style>
  .pane { display: flex; flex-direction: column; gap: 16px; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .hint { margin: 0; font-size: 13px; color: var(--text-3); }
  code { font-family: ui-monospace, monospace; font-size: 12px; padding: 2px 6px; border-radius: 6px; background: var(--bg-3); color: var(--text-2); }
  .list { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 12px; padding: 12px 0; border-bottom: 1px solid var(--line-soft); }
  .row strong { flex: 1; font-weight: 600; }
  .key { min-width: 160px; height: 36px; padding: 0 14px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-2); color: var(--text); font-weight: 600; letter-spacing: .02em; }
  .key.recording { border-color: var(--accent); color: var(--accent); }
  .key:disabled { opacity: .4; }
  .master > div { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .master small { line-height: 1.4; }
  .clear { width: 32px; height: 32px; border-radius: 50%; border: 0; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .clear:disabled { opacity: .3; }
  small { font-size: 12px; color: var(--text-3); }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .warn { margin: 0; padding: 10px 12px; border-radius: 10px; background: var(--warn-soft); color: var(--warn); font-size: 13px; }
</style>
