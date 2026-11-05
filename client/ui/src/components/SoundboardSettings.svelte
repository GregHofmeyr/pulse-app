<script lang="ts">
  // Server settings → Soundboard: every sound in this server, with who added it, how often it's played and how long
  // it is. Preview plays it only for you; Edit and Add sound use the soundboard's own dialog; Remove asks first.
  // Anyone in the server may do all of this (as in the soundboard itself).
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { isNotFound, previewPercents } from '../lib/soundboard'
  import { soundboard } from '../lib/soundboard.svelte'
  import { mixer } from '../lib/mixer.svelte'
  import { errorText, soundboardCmd } from '../lib/tauri'
  import { loadAudioConfig } from '../lib/voiceui'
  import type { Server } from '../lib/protocol/Server'
  import type { Sound } from '../lib/protocol/Sound'
  import SoundDialog from './SoundDialog.svelte'
  import SoundEmoji from './SoundEmoji.svelte'

  let { server }: { server: Server } = $props()

  const sounds = $derived(
    Object.values(app.state.sounds)
      .filter((s) => s.server_id === server.id)
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })),
  )

  let error = $state('')
  let dialog = $state<{ kind: 'add'; serverId: string } | { kind: 'edit'; sound: Sound } | null>(null)
  let confirming = $state<string | null>(null)
  let removing = false

  const seconds = (ms: number) => `${(ms / 1000).toFixed(1)} s`
  const plays = (n: number) => `${n} ${n === 1 ? 'play' : 'plays'}`
  const who = (s: Sound) => (s.added_by ? displayName(app.state, server.id, s.added_by) : 'Unknown')

  function preview(s: Sound) {
    error = ''
    const [calls, level] = previewPercents(mixer.calls.master, soundboard.volume_pct)
    soundboardCmd.play(s.media_hash, calls, level, s.volume, loadAudioConfig().output ?? null).catch((e) => (error = errorText(e)))
  }

  async function remove(s: Sound) {
    if (removing) return
    removing = true
    error = ''
    try {
      await soundboardCmd.remove(s.id)
      confirming = null
    } catch (e) {
      if (isNotFound(e)) confirming = null // someone else removed it first: same result
      else error = errorText(e)
    } finally {
      removing = false
    }
  }
</script>

<section class="settings-pane">
  <header>
    <h2>Soundboard</h2>
    <span class="count" aria-label="Sounds">{sounds.length}</span>
  </header>
  <p class="hint">Anyone in the server can add, edit and remove sounds. Sounds are up to 6 seconds.</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <button type="button" class="btn btn-secondary add" onclick={() => (dialog = { kind: 'add', serverId: server.id })}>Add sound</button>

  {#if sounds.length === 0}
    <p class="empty">No sounds yet.</p>
  {:else}
    <ul class="settings-list">
      {#each sounds as s (s.id)}
        <li class="settings-list-item">
          <span class="glyph"><SoundEmoji emoji={s.emoji} emojiId={s.emoji_id} size={24} /></span>
          <div class="meta">
            <span class="name">{s.name}</span>
            <small>{who(s)} · {plays(s.plays)} · {seconds(s.duration_ms)}</small>
          </div>
          {#if confirming === s.id}
            <span class="q">Remove {s.name}?</span>
            <button type="button" class="btn btn-secondary btn-danger-text" onclick={() => remove(s)}>Remove</button>
            <button type="button" class="btn btn-secondary" onclick={() => (confirming = null)}>Cancel</button>
          {:else}
            <button type="button" class="btn btn-secondary" aria-label="Preview {s.name}" onclick={() => preview(s)}>Preview</button>
            <button type="button" class="btn btn-secondary" aria-label="Edit {s.name}" onclick={() => (dialog = { kind: 'edit', sound: s })}>Edit</button>
            <button type="button" class="btn btn-secondary" aria-label="Remove {s.name}" onclick={() => (confirming = s.id)}>Remove</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if dialog}
  <SoundDialog mode={dialog} onClose={() => (dialog = null)} />
{/if}

<style>
  header { display: flex; align-items: baseline; gap: 12px; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .count { color: var(--text-3); font-size: 13px; }
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .empty { margin: 0; color: var(--text-3); }
  .add { align-self: flex-start; }
  /* the list is the shared .settings-list (app.css) */
  .glyph { width: 32px; height: 32px; display: grid; place-items: center; color: var(--text-2); flex-shrink: 0; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  small { color: var(--text-3); }
  .q { font-size: 13px; color: var(--text-2); }
</style>
