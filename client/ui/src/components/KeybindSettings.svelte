<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { actions, bindLabel, fromKeyEvent, keybindsApi, type Action, type Keybinds } from '../lib/keybinds'

  const linux = navigator.userAgent.includes('Linux')

  let binds = $state<Keybinds | null>(null)
  let recording = $state<Action | null>(null)
  let held = $state('')
  let error = $state('')
  let problems = $state<string[]>([])

  onMount(async () => {
    try {
      const r = await keybindsApi.get()
      binds = r.binds
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

  async function record(action: Action) {
    if (recording === action) return stop()
    error = ''
    held = ''
    recording = action
    // Windows would swallow the keys being recorded while they're registered globally
    await keybindsApi.pause(true).catch(() => {})
  }

  async function stop() {
    recording = null
    held = ''
    await keybindsApi.pause(false).catch(() => {})
  }

  async function onKey(e: KeyboardEvent) {
    if (!recording || !binds) return
    e.preventDefault()
    e.stopPropagation()
    const r = fromKeyEvent(e)
    if (r.kind === 'held') return void (held = r.label)
    // stop first: a second keydown (or auto-repeat) during the save mustn't save again
    const action = recording
    recording = null
    held = ''
    if (r.kind === 'bind') await save({ ...binds, [action]: r.bind })
    await keybindsApi.pause(false).catch(() => {})
  }

  onDestroy(() => {
    if (recording) void keybindsApi.pause(false).catch(() => {})
  })
</script>

<!-- leaving the window mid-recording ends it, or the hotkeys would stay released -->
<svelte:window onkeydowncapture={onKey} onblur={() => recording && stop()} />

<section class="pane">
  <h2>Keybinds</h2>
  {#if linux}
    <p class="hint">On Linux, apps can't grab keys for themselves: bind these commands in your desktop's keyboard settings (Hyprland: <code>bind = CTRL SHIFT, M, exec, pulse-app --toggle-mute</code>).</p>
    <div class="list">
      {#each actions as a}
        <div class="row"><strong>{a.label}</strong><code>pulse-app --{a.key.replace('_', '-')}</code></div>
      {/each}
    </div>
  {:else if binds}
    <p class="hint">Work everywhere, even when Pulse isn't focused or you're not in a call. Click a key to change it.</p>
    <div class="list">
      {#each actions as a}
        <div class="row">
          <strong>{a.label}</strong>
          <button class="key" class:recording={recording === a.key} aria-label="Change {a.label}"
            onclick={() => record(a.key)}>
            {recording === a.key ? held || 'Press keys…' : bindLabel(binds[a.key])}
          </button>
          <button class="clear" aria-label="Remove {a.label} bind" title="Remove" disabled={!binds[a.key] || !!recording}
            onclick={() => binds && save({ ...binds, [a.key]: null })}><Icon name="x" size={14} /></button>
        </div>
      {/each}
    </div>
    {#if recording}<small>Press the combination. Esc cancels.</small>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#each problems as p}<p class="warn" role="status">{p}</p>{/each}
  {/if}
</section>

<style>
  .pane { display: flex; flex-direction: column; gap: 16px; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .hint { margin: 0; font-size: 13px; color: var(--text-3); }
  code { font-family: ui-monospace, monospace; font-size: 12px; padding: 2px 6px; border-radius: 6px; background: var(--bg-3); color: var(--text-2); }
  .list { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 12px; padding: 12px 0; border-bottom: 1px solid #26282e; }
  .row strong { flex: 1; font-weight: 600; }
  .key { min-width: 160px; height: 36px; padding: 0 14px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-2); color: var(--text); font-weight: 600; letter-spacing: .02em; }
  .key.recording { border-color: var(--accent); color: var(--accent); }
  .clear { width: 32px; height: 32px; border-radius: 50%; border: 0; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .clear:disabled { opacity: .3; }
  small { font-size: 12px; color: var(--text-3); }
  .error { margin: 0; color: #f2616b; font-size: 13px; }
  .warn { margin: 0; padding: 10px 12px; border-radius: 10px; background: rgba(232, 176, 74, .12); color: #e8b04a; font-size: 13px; }
</style>
