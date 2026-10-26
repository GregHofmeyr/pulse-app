<script lang="ts">
  import { onDestroy } from 'svelte'
  import {
    SOUND_LABELS, SOUND_NAMES, SOUND_SETS, loadSoundPrefs, previewSet, saveSoundPrefs, stopPreview,
    type SoundName, type SoundSetId,
  } from '../lib/voiceui'

  let prefs = $state(loadSoundPrefs())

  onDestroy(stopPreview)

  function persist() {
    saveSoundPrefs($state.snapshot(prefs))
  }
  function choose(id: SoundSetId) {
    prefs.set = id
    persist()
  }
  function onKey(e: KeyboardEvent, id: SoundSetId) {
    if (e.key !== 'Enter' && e.key !== ' ') return
    e.preventDefault()
    choose(id)
  }
  function toggle(name: SoundName) {
    prefs.off = prefs.off.includes(name) ? prefs.off.filter((n) => n !== name) : [...prefs.off, name]
    persist()
  }
</script>

<h3 class="settings-label heading">Sounds</h3>
<div class="settings-field">
  <div class="sets" role="radiogroup" aria-label="Sound set">
    {#each SOUND_SETS as s (s.id)}
      <div class="set" class:selected={prefs.set === s.id}>
        <div class="pick" role="radio" aria-checked={prefs.set === s.id} tabindex="0"
          onclick={() => choose(s.id)} onkeydown={(e) => onKey(e, s.id)}>
          <strong>{s.label}</strong>
          <small>{s.blurb}</small>
        </div>
        <button class="play" aria-label="Preview {s.label}" title="Preview" onclick={() => previewSet(s.id)}>▶</button>
      </div>
    {/each}
  </div>
</div>

<label class="vol">SOUND VOLUME <strong>{prefs.volume_pct}%</strong>
  <input type="range" min="0" max="100" step="1" aria-label="Sound volume" bind:value={prefs.volume_pct} onchange={persist} />
  <small>Applies to every Pulse sound.</small>
</label>

<div class="events">
  {#each SOUND_NAMES as n (n)}
    <div class="row">
      <strong>{SOUND_LABELS[n]}</strong>
      <button class="switch" class:on={!prefs.off.includes(n)} aria-label={SOUND_LABELS[n]} aria-pressed={!prefs.off.includes(n)}
        onclick={() => toggle(n)}><span></span></button>
    </div>
  {/each}
</div>

<style>
  .heading { margin: 0; text-transform: uppercase; }
  .sets { display: flex; flex-direction: column; gap: 8px; }
  .set { display: flex; align-items: center; gap: 8px; padding-right: 12px; background: var(--bg-2); border: 1px solid transparent; border-radius: 12px; }
  .set.selected { border-color: var(--accent); }
  .pick { flex: 1; display: flex; flex-direction: column; gap: 2px; padding: 12px 14px; border-radius: 12px; cursor: pointer; }
  .set.selected .pick strong { color: var(--accent); }
  .play { width: 32px; height: 32px; flex-shrink: 0; border: 0; border-radius: 50%; background: var(--accent); color: var(--on-accent); font-size: 12px; }
  .vol { display: flex; flex-direction: column; gap: 8px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  .vol strong { color: var(--text); letter-spacing: 0; }
  input[type='range'] { accent-color: var(--accent); }
  .events { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 16px; padding: 12px 0; border-bottom: 1px solid #26282e; }
  .row strong { flex: 1; font-weight: 600; }
</style>
