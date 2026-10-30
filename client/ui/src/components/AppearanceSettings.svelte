<script lang="ts">
  import { appearance, setShowNameStyles } from '../lib/appearance.svelte'
  const uid = $props.id()
</script>

<section class="settings-pane">
  <h2>Appearance</h2>
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
</style>
