<script lang="ts">
  import Select from './Select.svelte'
  import { notifyPrefs, setNotificationsEnabled, setNotificationText } from '../lib/notifyPrefs.svelte'
  import { reminderPrefs, setReminderAlerts } from '../lib/reminderPrefs.svelte'
  import { setToastPref, TOAST_KINDS, toastPrefs } from '../lib/toastPrefs.svelte'
  import Avatar from './Avatar.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import Icon from './Icon.svelte'
  import PeoplePicker from './PeoplePicker.svelte'
  import { activePersonDnd, personDndUntilLabel } from '../lib/personDnd'
  import { setPersonDnd } from '../lib/personDnd.svelte'
  import type { PersonDnd } from '../lib/protocol/PersonDnd'
  const uid = $props.id()

  // Per-person Do Not Disturb (#310): the people whose messages don't alert you, by the name you see them by, in the
  // order you added them, each with where (everywhere or only in DMs), how long, and Remove. + Add person opens the
  // people picker (on everywhere until turned off); their right-click menu does it too, with a timer.
  const quiet = $derived(activePersonDnd(app.state.personDnd, clock.nowIso).map((p) => ({ ...p, name: displayName(app.state, null, p.user_id) })))
  const quietCandidates = $derived.by(() => {
    const s = app.state
    const ids = new Set([...Object.values(s.members).flatMap((ms) => ms.map((m) => m.user.id)), ...Object.values(s.dmMembers).flat()])
    ids.delete(s.me?.id ?? '')
    for (const p of quiet) ids.delete(p.user_id)
    return [...ids]
  })
  let pickingQuiet = $state(false)
  let quietError = $state('')
  async function quietDo(f: () => Promise<unknown>) {
    quietError = ''
    try {
      await f()
    } catch (e) {
      quietError = errorText(e)
    }
  }
  function addQuiet(ids: string[]) {
    pickingQuiet = false
    void quietDo(async () => {
      for (const id of ids) await setPersonDnd(id, { dms_only: false, until: null })
    })
  }
  const setWhere = (p: PersonDnd, dmsOnly: boolean) => quietDo(() => setPersonDnd(p.user_id, { dms_only: dmsOnly, until: p.until }))

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
  <div class="settings-field">
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={reminderPrefs.alerts} aria-describedby="{uid}-remind-help"
        onchange={(e) => setReminderAlerts(e.currentTarget.checked)} />
      <span>Reminder alerts</span>
    </label>
    <small id="{uid}-remind-help">When a message reminder comes due: the message sound, and a desktop notification while Pulse isn't in focus. Off: it only lands in your inbox.</small>
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

  <div class="settings-field">
    <h3 class="settings-section-title">Do Not Disturb for people</h3>
    <small>Their messages make no sound and no desktop notification, mentions included. They still arrive and count as unread. They're never told.</small>
    <button type="button" class="btn btn-secondary add" onclick={() => (pickingQuiet = true)}><Icon name="plus" size={14} /> Add person</button>
    {#if quiet.length}
      <ul class="quiet-list settings-list">
        {#each quiet as p (p.user_id)}
          <li class="settings-list-item">
            <Avatar id={p.user_id} name={p.name} size={28} />
            <span class="who-col"><span class="who">{p.name}</span><span class="until">{personDndUntilLabel(p.until, Date.parse(clock.nowIso))}</span></span>
            <span class="where"><Select size="sm" label="Where Do Not Disturb applies for {p.name}" value={p.dms_only ? 'dms' : 'all'}
              options={[{ value: 'all', label: 'Everywhere' }, { value: 'dms', label: 'Only in DMs' }]} onchange={(v) => void setWhere(p, v === 'dms')} /></span>
            <button type="button" class="btn btn-secondary" aria-label="Turn off Do Not Disturb for {p.name}" onclick={() => void quietDo(() => setPersonDnd(p.user_id, null))}>Remove</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">Nobody. Add someone here, or right-click them and pick "Do Not Disturb for them".</p>
    {/if}
    {#if quietError}<p class="error" role="alert">{quietError}</p>{/if}
  </div>
</section>

{#if pickingQuiet}
  <PeoplePicker title="Do Not Disturb for" hint="Their messages won't make a sound or a notification. They're never told." only={quietCandidates} max={Infinity}
    action="Turn on" onDone={addQuiet} onClose={() => (pickingQuiet = false)} />
{/if}

<style>
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  /* under the master switch, indented to its label */
  .kinds { display: flex; flex-direction: column; gap: 10px; margin-left: 54px; }
  .kinds .row { font-weight: 500; }
  .empty { margin: 0; color: var(--text-3); font-size: 13px; }
  /* the list is the shared .settings-list (app.css) */
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .add { align-self: flex-start; }
  .who-col { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .until { color: var(--text-3); font-size: 12px; }
  .where { width: 140px; flex-shrink: 0; }
</style>
