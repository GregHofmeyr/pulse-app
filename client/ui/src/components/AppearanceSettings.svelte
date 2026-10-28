<script lang="ts">
  import { appearance, setShowNameStyles } from '../lib/appearance.svelte'
  const uid = $props.id()
</script>

<section class="settings-pane">
  <h2>Appearance</h2>
  <div class="settings-field">
    <label class="row">
      <input type="checkbox" role="switch" checked={appearance.showNameStyles} aria-describedby="{uid}-help"
        onchange={(e) => setShowNameStyles(e.currentTarget.checked)} />
      <span>Show name styles</span>
    </label>
    <small id="{uid}-help">Show everyone's chosen name fonts and colours. Off shows plain names (role colours still show).</small>
  </div>
</section>

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  /* a checkbox drawn as a switch */
  input[role='switch'] { appearance: none; width: 34px; height: 20px; margin: 0; border-radius: 10px; background: var(--bg-4); position: relative; flex-shrink: 0; cursor: pointer; transition: background .12s; }
  input[role='switch']::after { content: ''; position: absolute; top: 3px; left: 3px; width: 14px; height: 14px; border-radius: 50%; background: #fff; transition: transform .12s; }
  input[role='switch']:checked { background: var(--accent); }
  input[role='switch']:checked::after { transform: translateX(14px); }
  input[role='switch']:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @media (prefers-reduced-motion: reduce) { input[role='switch'], input[role='switch']::after { transition: none; } }
</style>
