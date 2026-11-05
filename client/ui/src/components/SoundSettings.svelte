<script lang="ts">
  import { onDestroy } from 'svelte'
  import Icon from './Icon.svelte'
  import {
    SOUND_SETS, loadSoundPrefs, previewSet, saveSoundPrefs, stopPreview, type SoundSetId,
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
  let rows = $state<HTMLElement[]>([])
  const STEP: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }
  function onKey(e: KeyboardEvent, i: number) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      choose(SOUND_SETS[i].id)
    } else if (Object.hasOwn(STEP, e.key)) {
      e.preventDefault()
      const j = (i + STEP[e.key] + SOUND_SETS.length) % SOUND_SETS.length
      choose(SOUND_SETS[j].id)
      rows[j]?.focus()
    }
  }
</script>

<h3 class="settings-section-title">Sounds</h3>
<div class="settings-field">
  <div class="sets" role="radiogroup" aria-label="Sound set">
    {#each SOUND_SETS as s, i (s.id)}
      <div class="set" class:selected={prefs.set === s.id}>
        <div class="pick" role="radio" aria-checked={prefs.set === s.id} tabindex={prefs.set === s.id ? 0 : -1}
          aria-label={s.label} aria-describedby="sound-set-{s.id}-blurb" bind:this={rows[i]}
          onclick={() => choose(s.id)} onkeydown={(e) => onKey(e, i)}>
          <strong>{s.label}</strong>
          <small id="sound-set-{s.id}-blurb">{s.blurb}</small>
        </div>
        <button class="play btn-primary" aria-label="Preview {s.label}" data-tip="Preview" onclick={() => previewSet(s.id)}><Icon name="play" size={14} /></button>
      </div>
    {/each}
  </div>
</div>

<small class="hint">How loud each sound plays is in the Volume mixer below.</small>

<style>
  .sets { display: flex; flex-direction: column; gap: 8px; }
  .set { display: flex; align-items: center; gap: 8px; padding-right: 12px; background: var(--bg-2); border: 1px solid transparent; border-radius: var(--radius-lg); }
  .set.selected { border-color: var(--focus-ring); }
  .pick { flex: 1; display: flex; flex-direction: column; gap: 2px; padding: 12px 14px; border-radius: var(--radius-lg); cursor: pointer; }
  .set.selected .pick strong { color: var(--accent-text); }
  .play { width: 32px; height: 32px; flex-shrink: 0; border: 0; border-radius: 50%; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; }
  .hint { margin-top: -8px; font-size: 12px; color: var(--text-3); }
</style>
