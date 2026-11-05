<script lang="ts">
  import { onDestroy } from 'svelte'
  import Icon from './Icon.svelte'
  import {
    RINGTONES, SOUND_SETS, loadSoundPrefs, previewRingtone, previewSet, saveSoundPrefs, stopPreview,
    type RingtoneId, type SoundSetId,
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
  function chooseRingtone(id: RingtoneId) {
    prefs.ringtone = id
    persist()
  }
  let rows = $state<HTMLElement[]>([])
  let toneRows = $state<HTMLElement[]>([])
  const STEP: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }
  /** A roving-tabindex radiogroup: Space/Enter pick row `i`, arrows pick and focus the next one (wrapping). */
  function onKey<T>(e: KeyboardEvent, i: number, ids: readonly T[], pick: (id: T) => void, els: HTMLElement[]) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      pick(ids[i])
    } else if (Object.hasOwn(STEP, e.key)) {
      e.preventDefault()
      const j = (i + STEP[e.key] + ids.length) % ids.length
      pick(ids[j])
      els[j]?.focus()
    }
  }
  const setIds = SOUND_SETS.map((s) => s.id)
  const toneIds = RINGTONES.map((t) => t.id)
</script>

<h3 class="settings-section-title">Sounds</h3>
<div class="settings-field">
  <div class="sets" role="radiogroup" aria-label="Sound set">
    {#each SOUND_SETS as s, i (s.id)}
      <div class="set" class:selected={prefs.set === s.id}>
        <div class="pick" role="radio" aria-checked={prefs.set === s.id} tabindex={prefs.set === s.id ? 0 : -1}
          aria-label={s.label} aria-describedby="sound-set-{s.id}-blurb" bind:this={rows[i]}
          onclick={() => choose(s.id)} onkeydown={(e) => onKey(e, i, setIds, choose, rows)}>
          <strong>{s.label}</strong>
          <small id="sound-set-{s.id}-blurb">{s.blurb}</small>
        </div>
        <button class="play btn-primary" aria-label="Preview {s.label}" data-tip="Preview" onclick={() => previewSet(s.id)}><Icon name="play" size={14} /></button>
      </div>
    {/each}
  </div>
</div>

<!-- #174: what incoming calls ring with, on this device; its loudness is the mixer's Ringing notch, just below -->
<h3 class="settings-label heading">Ringtone</h3>
<div class="settings-field">
  <div class="sets" role="radiogroup" aria-label="Ringtone">
    {#each RINGTONES as t, i (t.id)}
      <div class="set" class:selected={prefs.ringtone === t.id}>
        <div class="pick" role="radio" aria-checked={prefs.ringtone === t.id} tabindex={prefs.ringtone === t.id ? 0 : -1}
          aria-label={t.label} aria-describedby="ringtone-{t.id}-blurb" bind:this={toneRows[i]}
          onclick={() => chooseRingtone(t.id)} onkeydown={(e) => onKey(e, i, toneIds, chooseRingtone, toneRows)}>
          <strong>{t.label}</strong>
          <small id="ringtone-{t.id}-blurb">{t.blurb}</small>
        </div>
        <button class="play" aria-label="Preview ringtone {t.label}" title="Preview" onclick={() => previewRingtone(t.id)}><Icon name="play" size={14} /></button>
      </div>
    {/each}
  </div>
</div>

<small class="hint">How loud each sound plays, and the ringtone (Ringing), is in the Volume mixer below.</small>

<style>
  .sets { display: flex; flex-direction: column; gap: 8px; }
  .set { display: flex; align-items: center; gap: 8px; padding-right: 12px; background: var(--bg-2); border: 1px solid transparent; border-radius: var(--radius-lg); }
  .set.selected { border-color: var(--focus-ring); }
  .pick { flex: 1; display: flex; flex-direction: column; gap: 2px; padding: 12px 14px; border-radius: var(--radius-lg); cursor: pointer; }
  .set.selected .pick strong { color: var(--accent-text); }
  .play { width: 32px; height: 32px; flex-shrink: 0; border: 0; border-radius: 50%; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; }
  .hint { margin-top: -8px; font-size: 12px; color: var(--text-3); }
</style>
