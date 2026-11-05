<script lang="ts">
  import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'
  import { setToastPref, TOAST_KINDS, toastPrefs } from '../lib/toastPrefs.svelte'
  const uid = $props.id()
</script>

<section class="settings-pane">
  <h2>Notifications</h2>
  <div class="settings-field">
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={notifyPrefs.enabled} aria-describedby="{uid}-on-help"
        onchange={(e) => setNotificationsEnabled(e.currentTarget.checked)} />
      <span>Desktop notifications</span>
    </label>
    <small id="{uid}-on-help">While Pulse isn't in focus, a message that would ding (a DM, a group, a mention) also shows a system notification. Click it to open the conversation.</small>
  </div>
  <div class="settings-field">
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={notifyPrefs.showText} disabled={!notifyPrefs.enabled} aria-describedby="{uid}-text-help"
        onchange={(e) => setNotificationText(e.currentTarget.checked)} />
      <span>Show message text</span>
    </label>
    <small id="{uid}-text-help">Off: notifications say who and where, but only "New message" or "Mentioned you".</small>
  </div>

  <!-- in-app toasts (#166): their own prefs, apart from desktop notifications -->
  <div class="settings-field">
    <h3>In-app toasts</h3>
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={toastPrefs.enabled} onchange={(e) => setToastPref('enabled', e.currentTarget.checked)} />
      <span>Show in-app toasts</span>
    </label>
    <div class="kinds">
      {#each TOAST_KINDS as k (k.kind)}
        <label class="row">
          <input type="checkbox" role="switch" class="switch" checked={toastPrefs[k.kind]} disabled={!toastPrefs.enabled}
            onchange={(e) => setToastPref(k.kind, e.currentTarget.checked)} />
          <span>{k.label}</span>
        </label>
      {/each}
    </div>
  </div>
</section>

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  h3 { margin: 8px 0 2px; font-size: 15px; }
  /* under the master switch, indented to its label */
  .kinds { display: flex; flex-direction: column; gap: 10px; margin-left: 54px; }
  .kinds .row { font-weight: 500; }
</style>
