<script lang="ts">
  // Settings → Privacy (#45): the people you appear offline to, each with Remove. Add person (#188) opens the people
  // picker with anyone you could hide from; their right-click menu ("Appear offline to them") does it too. Private to
  // you, like private nicknames: they're never told.
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import PeoplePicker from './PeoplePicker.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { activityPrefs, mediaActivity, setReadMedia, setShowActivity } from '../lib/activityPrefs.svelte'
  import { songLine } from '../lib/activity'
  const uid = $props.id()

  let error = $state('')
  // the name you see them by (#47); the list keeps the order you hid them in
  const hidden = $derived(app.state.appearOfflineTo.map((id) => ({ id, name: displayName(app.state, null, id) })))

  // Anyone you could be seen by: sharing a server, DM or group with you; never you, nor anyone already on the list.
  const candidates = $derived.by(() => {
    const s = app.state
    const ids = new Set([...Object.values(s.members).flatMap((ms) => ms.map((m) => m.user.id)), ...Object.values(s.dmMembers).flat()])
    ids.delete(s.me?.id ?? '')
    for (const id of s.appearOfflineTo) ids.delete(id)
    return [...ids]
  })
  let picking = $state(false)
  async function add(ids: string[]) {
    picking = false
    error = ''
    try {
      // each AppearOfflineChanged that follows puts them on the list
      for (const id of ids) await api.setAppearOffline(id, true)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function remove(id: string) {
    error = ''
    try {
      // the AppearOfflineChanged that follows takes them off the list
      await api.setAppearOffline(id, false)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<section class="settings-pane">
  <h2>Privacy</h2>
  <!-- "Listening to" (#275): both off until turned on; reading off means Pulse runs nothing for it -->
  <div class="settings-field">
    <h3 class="settings-section-title">Activity</h3>
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={activityPrefs.read} aria-describedby="{uid}-read-help"
        onchange={(e) => setReadMedia(e.currentTarget.checked)} />
      <span>Read media activity</span>
    </label>
    <small id="{uid}-read-help">Lets Pulse see what the Spotify app on this computer is playing. Off, Pulse doesn't look at all.</small>
    <label class="row">
      <input type="checkbox" role="switch" class="switch" checked={activityPrefs.show} disabled={!activityPrefs.read} aria-describedby="{uid}-show-help"
        onchange={(e) => setShowActivity(e.currentTarget.checked)} />
      <span>Show my activity</span>
    </label>
    <small id="{uid}-show-help">Everyone who can see you online sees "Listening to" on your profile. Off, it only updates a Jam you share to the stage.</small>
    {#if activityPrefs.read}
      <p class="reading" aria-live="polite">{mediaActivity.playing ? `Spotify is playing ${songLine(mediaActivity.playing)}` : 'Nothing playing in Spotify right now.'}</p>
    {/if}
  </div>
  <div class="settings-field">
    <h3 class="settings-section-title">Appear offline to</h3>
    <p class="hint">They see you offline, whatever your status, with your last seen frozen at when you hid. They're never told.</p>
    <button type="button" class="btn btn-secondary add" onclick={() => (picking = true)}><Icon name="plus" size={14} /> Add person</button>
    {#if hidden.length}
      <ul class="hidden-list settings-list">
        {#each hidden as p (p.id)}
          <li class="settings-list-item">
            <Avatar id={p.id} name={p.name} size={28} />
            <span class="who">{p.name}</span>
            <button type="button" class="btn btn-secondary" aria-label="Stop appearing offline to {p.name}" onclick={() => remove(p.id)}>Remove</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">You don't appear offline to anyone. Add someone here, or right-click them and pick "Appear offline to them".</p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
</section>

{#if picking}
  <PeoplePicker title="Appear offline to" hint="They'll see you offline. They're never told." only={candidates} max={Infinity}
    action="Appear offline" onDone={add} onClose={() => (picking = false)} />
{/if}

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  /* the list is the shared .settings-list (app.css) */
  .add { align-self: flex-start; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .reading { color: var(--text-3); }
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
