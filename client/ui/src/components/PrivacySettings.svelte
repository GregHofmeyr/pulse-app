<script lang="ts">
  // Settings → Privacy (#45): the people you appear offline to, each with Remove. Adding someone is in their
  // right-click menu ("Appear offline to them"). Private to you, like private nicknames: they're never told.
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'

  let error = $state('')
  // the name you see them by (#47); the list keeps the order you hid them in
  const hidden = $derived(app.state.appearOfflineTo.map((id) => ({ id, name: displayName(app.state, null, id) })))

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
  <div class="settings-field">
    <h3 class="settings-section-title">Appear offline to</h3>
    <p class="hint">They see you offline, whatever your status, with your last seen frozen at when you hid. They're never told.</p>
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
      <p class="empty">You don't appear offline to anyone. Right-click someone and pick "Appear offline to them".</p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
</section>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  /* the list is the shared .settings-list (app.css) */
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
