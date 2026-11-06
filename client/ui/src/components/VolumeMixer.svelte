<script lang="ts">
  // Settings → Voice & Audio → Volume mixer: a slider for everything Pulse plays, in sections. Each section's master
  // multiplies its items (and per-person / per-stream volumes multiply those, in the core).
  import Icon from './Icon.svelte'
  import { MIXER_SECTIONS, mixGain, type MixerSection, type Notch } from '../lib/mixer'
  import { mixer, resetMixerSection, setNotch } from '../lib/mixer.svelte'
  import { mixerCollapsed, toggleMixerSection } from '../lib/mixerCollapsed.svelte'
  import { memberServers } from '../lib/reactions'
  import { soundSections } from '../lib/soundboard'
  import { app } from '../lib/store.svelte'
  import { soundboardCmd } from '../lib/tauri'
  import { loadAudioConfig, playRingingAt, playSoundAt, type SoundName } from '../lib/voiceui'

  const value = (s: MixerSection, key: string): number => (mixer[s.id] as Record<string, number>)[key]
  const label = (pct: number) => (pct === 0 ? 'Off' : `${pct}%`)

  /** What the Soundboard ▶ plays: the first sound the popover would show. */
  const firstSound = $derived(soundSections(app.state, memberServers(app.state), app.state.servers).find((x) => x.sounds.length)?.sounds[0] ?? null)

  /** The level a test button would play at (0: off, so the button is disabled). */
  const testGain = (s: MixerSection, n: Notch): number =>
    n.test === 'soundboard' ? mixGain(mixer.calls.master, mixer.calls.soundboard) : mixGain(value(s, 'master'), value(s, n.key))
  const testBlocked = (s: MixerSection, n: Notch): string | null =>
    n.test === 'soundboard' && !firstSound ? 'No sounds yet' : testGain(s, n) > 0 ? null : 'Off'

  function test(s: MixerSection, n: Notch) {
    if (n.test === 'sound') {
      playSoundAt(n.key as SoundName, testGain(s, n))
    } else if (n.test === 'ring') {
      playRingingAt(testGain(s, n))
    } else if (n.test === 'soundboard' && firstSound) {
      const { master, soundboard } = mixer.calls
      soundboardCmd.play(firstSound.media_hash, master, soundboard, firstSound.volume, loadAudioConfig().output ?? null).catch(() => {})
    }
  }
</script>

<h3 class="settings-section-title">Volume mixer</h3>
<p class="lede">Each section's top slider turns everything under it up or down together. 0% is off.</p>

<div class="sections">
  {#each MIXER_SECTIONS as s (s.id)}
    {@const shut = mixerCollapsed.ids.includes(s.id)}
    <section class="mix-section" aria-labelledby="mix-{s.id}">
      <div class="head">
        <!-- the heading's button folds the rows away (#164); the master and Reset stay beside it -->
        <h4 id="mix-{s.id}"><button type="button" class="fold" class:shut aria-expanded={!shut} aria-controls="mix-{s.id}-rows"
          onclick={() => toggleMixerSection(s.id)}><Icon name="chevron" size={14} /><span class="label">{s.label}</span></button></h4>
        {#if s.master}
          <input class="master" type="range" min="0" max="200" step="1" aria-label="{s.label} volume" aria-valuetext={label(value(s, 'master'))}
            value={value(s, 'master')} oninput={(e) => setNotch(s.id, 'master', +e.currentTarget.value)} />
          <strong class="pct">{label(value(s, 'master'))}</strong>
        {:else}
          <!-- one slider only (Media: Clips, which can't go past a video's own 100%): a section slider would repeat it -->
          <small class="solo">One slider, below</small>
        {/if}
        <button class="btn btn-quiet reset" aria-label="Reset {s.label} to defaults" data-tip="Reset to defaults" onclick={() => resetMixerSection(s.id)}>Reset</button>
      </div>
      <div class="rows" id="mix-{s.id}-rows" hidden={shut}>
      {#each s.notches as n (n.key)}
        <div class="row">
          <span class="name">{n.label}</span>
          <input type="range" min="0" max={n.max} step="1" aria-label={n.label} aria-valuetext={label(value(s, n.key))}
            value={value(s, n.key)} oninput={(e) => setNotch(s.id, n.key, +e.currentTarget.value)} />
          <span class="pct">{label(value(s, n.key))}</span>
          {#if n.test}
            <button class="play btn-primary" aria-label="Test {n.label}" data-tip={testBlocked(s, n) ?? 'Test'}
              disabled={testBlocked(s, n) !== null} onclick={() => test(s, n)}><Icon name="play" size={12} /></button>
          {:else}
            <span class="play-gap" aria-hidden="true"></span>
          {/if}
        </div>
      {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .lede { margin: -8px 0 0; font-size: 12px; color: var(--text-3); }
  .sections { display: flex; flex-direction: column; gap: 12px; }
  .mix-section { display: flex; flex-direction: column; gap: 2px; padding: 12px 14px; background: var(--bg-2); border-radius: var(--radius-lg); }
  .head { display: grid; grid-template-columns: minmax(0, 9rem) minmax(0, 1fr) 3.5em auto; align-items: center; gap: 10px; padding-bottom: 6px; }
  .head h4 { margin: 0; font-size: 14px; font-weight: 700; color: var(--text); grid-column: 1; min-width: 0; }
  /* one line, whatever the width: "Stream sounds" wrapped and made its row taller than the rest (#173) */
  .fold { display: flex; align-items: center; gap: 6px; max-width: 100%; white-space: nowrap; margin: -4px -6px; padding: 4px 6px; border: 0; border-radius: var(--radius-sm); background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .fold:hover { background: var(--bg-3); }
  .fold:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .fold :global(svg) { flex-shrink: 0; color: var(--text-3); transition: transform var(--dur) var(--ease); }
  .fold.shut :global(svg) { transform: rotate(-90deg); }
  .fold .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .solo { grid-column: 2 / 4; font-size: 12px; color: var(--text-3); }
  .rows[hidden] { display: none; }
  .head .reset { grid-column: 4; padding: 4px 10px; font-size: 12px; }
  .row { display: grid; grid-template-columns: minmax(0, 9rem) minmax(0, 1fr) 3.5em 28px; align-items: center; gap: 10px; padding: 6px 0; border-top: 1px solid var(--line-soft); }
  .name { font-size: 13px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pct { font-size: 12px; text-align: right; color: var(--text-2); font-variant-numeric: tabular-nums; }
  strong.pct { color: var(--accent-text); }
  input[type='range'] { width: 100%; margin: 0; accent-color: var(--accent); }
  .play { width: 28px; height: 28px; border: 0; border-radius: 50%; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; cursor: pointer; }
  .play:disabled { opacity: .4; cursor: default; }
  .play:focus-visible, .reset:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 520px) {
    .head, .row { grid-template-columns: minmax(0, 6rem) minmax(0, 1fr) 3em auto; gap: 8px; }
  }
</style>
