<script lang="ts">
  import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'
  import { setToastPref, TOAST_KINDS, toastPrefs } from '../lib/toastPrefs.svelte'
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  const uid = $props.id()

  // "Notify me when they go live" (#60): the people you follow, by the name you see them by, in the order you added
  // them. Adding someone is in their right-click menu; the list is on your account, so every device shows it.
  const following = $derived(app.state.liveNotify.map((id) => ({ id, name: displayName(app.state, null, id) })))
  let followError = $state('')
  async function unfollow(id: string) {
    followError = ''
    try {
      // the LiveNotifyChanged that follows takes them off the list
      await api.setLiveNotify(id, false)
    } catch (e) {
      followError = errorText(e)
    }
  }
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
    <h3 class="settings-section-title">In-app toasts</h3>
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

  <div class="settings-field">
    <h3 class="settings-section-title">Go-live notifications</h3>
    <small>When one of these people goes live anywhere you can see, you get a notification and the live toast. Do Not Disturb holds them back.</small>
    {#if following.length}
      <ul class="follow-list settings-list">
        {#each following as p (p.id)}
          <li class="settings-list-item">
            <Avatar id={p.id} name={p.name} size={28} />
            <span class="who">{p.name}</span>
            <button type="button" class="btn btn-secondary" aria-label="Stop notifying me when {p.name} goes live" onclick={() => unfollow(p.id)}>Remove</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">Nobody yet. Right-click someone and pick "Notify me when they go live".</p>
    {/if}
    {#if followError}<p class="error" role="alert">{followError}</p>{/if}
  </div>
</section>

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  /* under the master switch, indented to its label */
  .kinds { display: flex; flex-direction: column; gap: 10px; margin-left: 54px; }
  .kinds .row { font-weight: 500; }
  .empty { margin: 0; color: var(--text-3); font-size: 13px; }
  /* the list is the shared .settings-list (app.css) */
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
