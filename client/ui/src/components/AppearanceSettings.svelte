<script lang="ts">
  import { appearance, setShowNameStyles } from '../lib/appearance.svelte'
  import { nameOpts, setAccent, setNameReadability, setTheme, setTint, theme } from '../lib/theme.svelte'
  import { ACCENT_PRESETS, NAME_LEVELS, themeByKey, THEMES, TINTS } from '../lib/themes'
  import { NAME_GRADIENTS, NAME_SOLIDS, nameCss, resolveNameStyle, type NamePreset } from '../lib/nameStyle'
  import type { User } from '../lib/protocol/User'
  const uid = $props.id()

  const current = $derived(themeByKey(theme.key))
  // The colour well shows the custom accent, or the theme's own until one is picked.
  const shownAccent = $derived(theme.accent ?? current.accent)
  const presetPicked = (c: string) => theme.accent === c

  const level = $derived(Math.max(0, NAME_LEVELS.findIndex((l) => l.key === theme.names)))
  const levelText = (i: number) => `${NAME_LEVELS[i].label} (${NAME_LEVELS[i].ratio}:1)`

  // A few name styles as they'd look in chat on this theme, at this level: a light solid and two gradients (presets),
  // and a dark role colour.
  const person = (username: string, font: string | null, colors: string[]) => ({ id: username, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font, colors } }) as User
  const preset = (list: NamePreset[], key: string) => list.find((p) => p.key === key)!.colors
  const SAMPLES: { user: User; role: string | null }[] = [
    { user: person('alex', null, preset(NAME_SOLIDS, 'sky')), role: null },
    { user: person('sam', 'fredoka', preset(NAME_GRADIENTS, 'sunset')), role: null },
    { user: person('kim', 'caveat', preset(NAME_GRADIENTS, 'mint')), role: null },
    { user: person('jo', 'space-grotesk', []), role: '#3b4bd8' },
  ]
  const samples = $derived(SAMPLES.map((s) => ({ name: s.user.display_name, css: nameCss(resolveNameStyle(s.user, { roleColor: s.role, showStyles: true, ...nameOpts('chat') })) })))
</script>

<section class="settings-pane">
  <h2>Appearance</h2>

  <div class="settings-field">
    <span class="settings-label" id="{uid}-theme">THEME</span>
    <div class="themes" role="group" aria-label="Theme">
      {#each THEMES as t (t.key)}
        <button type="button" class="theme" aria-pressed={theme.key === t.key} onclick={() => setTheme(t.key)}>
          <span class="preview" data-theme={t.key} data-tint={t.key === 'light' ? theme.tint : undefined} aria-hidden="true">
            <span class="p-side"><i></i><i></i><i class="on"></i></span>
            <span class="p-chat"><i class="who"></i><i class="l1"></i><i class="l2"></i><span class="p-btn"></span></span>
          </span>
          <span class="t-label">{t.label}</span>
        </button>
      {/each}
    </div>
    <small>{current.blurb}</small>
  </div>

  {#if theme.key === 'light'}
    <div class="settings-field">
      <span class="settings-label">TINT</span>
      <div class="swatches" role="group" aria-label="Tint">
        {#each TINTS as n (n.key)}
          <button type="button" class="tint toggle-chip" aria-pressed={theme.tint === n.key} onclick={() => setTint(n.key)}>
            <span class="dot" data-theme="light" data-tint={n.key} aria-hidden="true"></span>{n.label}
          </button>
        {/each}
      </div>
      <small>A very light wash on the window, sidebar and lines. Panels stay white.</small>
    </div>
  {/if}

  <div class="settings-field">
    <span class="settings-label">ACCENT</span>
    <div class="swatches" role="group" aria-label="Accent">
      <button type="button" class="default toggle-chip" aria-pressed={theme.accent === null} onclick={() => setAccent(null)}>
        <span class="chip" style:background={current.accent} aria-hidden="true"></span>Theme default
      </button>
      {#each ACCENT_PRESETS as p (p.color)}
        <button type="button" class="chip-btn" aria-label={p.label} aria-pressed={presetPicked(p.color)} onclick={() => setAccent(p.color)}>
          <span class="chip" style:background={p.color} aria-hidden="true"></span>
        </button>
      {/each}
      <label class="custom" data-tip="Any colour">
        <input type="color" value={shownAccent} aria-describedby="{uid}-accent" oninput={(e) => setAccent(e.currentTarget.value)} />
        <span>Custom</span>
      </label>
    </div>
    <small id="{uid}-accent">Buttons, links, mentions and focus follow it. Text on it turns dark or white to stay readable.</small>
  </div>

  <div class="settings-field">
    <label class="settings-label" for="{uid}-names">NAME READABILITY</label>
    <div class="level">
      <input id="{uid}-names" type="range" min="0" max={NAME_LEVELS.length - 1} step="1" value={level} aria-valuetext={levelText(level)} aria-describedby="{uid}-names-help"
        oninput={(e) => setNameReadability(NAME_LEVELS[Number(e.currentTarget.value)].key)} />
      <div class="ticks" aria-hidden="true">{#each NAME_LEVELS as l, i (l.key)}<span class:on={i === level}>{l.label}</span>{/each}</div>
    </div>
    <div class="sample" aria-label="Sample names">
      {#each samples as s (s.name)}<span class="name" style={s.css}>{s.name}</span>{/each}
    </div>
    <small id="{uid}-names-help">How far everyone's name colours are moved to stand out from the background. Higher reads more easily; lower keeps colours closer to what people picked.</small>
  </div>

  <div class="settings-field">
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={appearance.showNameStyles} aria-describedby="{uid}-help"
        onchange={(e) => setShowNameStyles(e.currentTarget.checked)} />
      <span>Show name styles</span>
    </label>
    <small id="{uid}-help">Show everyone's chosen name fonts and colours. Off shows plain names (role colours still show).</small>
  </div>
</section>

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }

  /* Theme cards: a small live slice of each theme (its own data-theme, so its own tokens). */
  .themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(132px, 1fr)); gap: 10px; }
  .theme { display: flex; flex-direction: column; gap: 6px; padding: 6px; border: 2px solid transparent; border-radius: var(--radius-lg); background: transparent; color: var(--text-2); text-align: left; }
  .theme:hover { background: var(--bg-3); color: var(--text); }
  .theme[aria-pressed='true'] { border-color: var(--focus-ring); color: var(--text); }
  .t-label { font-size: 13px; font-weight: 600; padding: 0 2px 2px; }
  .preview { display: grid; grid-template-columns: 34% 1fr; gap: 4px; height: 72px; padding: 4px; border-radius: var(--radius-sm); background: var(--bg-0); border: 1px solid var(--line); overflow: hidden; }
  .preview i { display: block; height: 5px; border-radius: 3px; }
  .p-side { display: flex; flex-direction: column; gap: 5px; padding: 6px 5px; border-radius: 5px; background: var(--bg-1); }
  .p-side i { background: var(--text-3); opacity: .55; }
  .p-side i.on { background: var(--bg-3); opacity: 1; height: 8px; }
  .p-chat { position: relative; display: flex; flex-direction: column; gap: 5px; padding: 7px 6px; border-radius: 5px; background: var(--bg-2); box-shadow: var(--shadow-1); }
  .p-chat .who { width: 40%; background: var(--accent-text); }
  .p-chat .l1 { width: 85%; background: var(--text-body); opacity: .8; }
  .p-chat .l2 { width: 60%; background: var(--text-3); opacity: .7; }
  .p-btn { position: absolute; right: 6px; bottom: 6px; width: 26px; height: 10px; border-radius: 4px; background: var(--accent); }

  .swatches { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  /* the tints and Theme default are the shared .toggle-chip (app.css), with a swatch inside */
  .tint, .default { padding-left: 6px; }
  /* the tint's own ground (a preview of Light with that tint) */
  .tint .dot { width: 16px; height: 16px; border-radius: 50%; background: var(--bg-0); border: 1px solid var(--line-strong); }
  .chip { display: block; width: 18px; height: 18px; border-radius: 50%; }
  .chip-btn { display: grid; place-items: center; width: 32px; height: 32px; padding: 0; border: 2px solid transparent; border-radius: 50%; background: transparent; }
  .chip-btn .chip { width: 22px; height: 22px; }
  .chip-btn:hover { border-color: var(--line-strong); }
  .chip-btn[aria-pressed='true'] { border-color: var(--text); }
  .custom { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 600; color: var(--text-2); cursor: pointer; }
  .custom input { width: 32px; height: 32px; padding: 0; border: 2px solid var(--line); border-radius: var(--radius-sm); background: transparent; cursor: pointer; }

  .level { display: flex; flex-direction: column; gap: 4px; max-width: 320px; }
  .level input { width: 100%; margin: 0; accent-color: var(--accent); }
  .ticks { display: flex; justify-content: space-between; font-size: 12px; color: var(--text-3); }
  .ticks .on { color: var(--text); font-weight: 600; }
  .sample { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px 18px; padding: 12px 14px; border: 1px solid var(--line); border-radius: var(--radius-md); background: var(--bg-2); font-weight: 600; font-size: 15px; }
  .sample .name { display: inline-block; font-size: calc(1em * var(--name-scale, 1)); line-height: calc(1.4em / var(--name-scale, 1)); }
</style>
