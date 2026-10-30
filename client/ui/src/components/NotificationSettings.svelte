<script lang="ts">
  import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'
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
</section>

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
</style>
