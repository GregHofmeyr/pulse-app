<script lang="ts">
  // Server settings → Notifications (#203): your level for this server, the same choices as the server tab's
  // right-click menu (#195) with yours ticked, as a radio group like Sound settings' (roving focus, arrows pick).
  // Yours only: nobody else's changes. A channel's own level (its right-click menu or the bell) wins over this one.
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { serverLevel } from '../lib/conversations'
  import { NOTIFY_LEVELS, serverLevelToSet } from '../lib/notifyLevels'
  import type { NotifyLevel } from '../lib/protocol/NotifyLevel'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()

  const BLURB: Record<NotifyLevel, string> = {
    all: 'Every new message in its channels plays the message sound and, while Pulse isn’t in focus, notifies you.',
    mentions: 'Only messages that mention you.',
    nothing: 'No sound or notification for its messages. Unread counts still show.',
  }
  const uid = $props.id()
  const level = $derived(serverLevel(app.state, server.id))
  let error = $state('')
  let rows = $state<HTMLElement[]>([])

  async function choose(l: NotifyLevel) {
    error = ''
    try {
      await api.setNotifyLevel('server', server.id, serverLevelToSet(l))
    } catch (e) {
      error = errorText(e)
    }
  }
  const STEP: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }
  function onKey(e: KeyboardEvent, i: number) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      void choose(NOTIFY_LEVELS[i].level)
    } else if (Object.hasOwn(STEP, e.key)) {
      e.preventDefault()
      const j = (i + STEP[e.key] + NOTIFY_LEVELS.length) % NOTIFY_LEVELS.length
      void choose(NOTIFY_LEVELS[j].level)
      rows[j]?.focus()
    }
  }
</script>

<section class="settings-pane">
  <h2>Notifications</h2>
  <div class="settings-field">
    <div class="levels" role="radiogroup" aria-label="Notifications for {server.name}" aria-describedby="{uid}-help">
      {#each NOTIFY_LEVELS as l, i (l.level)}
        <div class="level" class:selected={level === l.level}>
          <div class="pick" role="radio" aria-checked={level === l.level} tabindex={level === l.level ? 0 : -1}
            aria-label={l.label} aria-describedby="{uid}-{l.level}" bind:this={rows[i]}
            onclick={() => choose(l.level)} onkeydown={(e) => onKey(e, i)}>
            <strong>{l.label}</strong>
            <small id="{uid}-{l.level}">{BLURB[l.level]}</small>
          </div>
          {#if level === l.level}<Icon name="check" size={16} />{/if}
        </div>
      {/each}
    </div>
    <small id="{uid}-help">Just for you. A channel set to its own level (right-click it, or its bell) keeps that one.</small>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
</section>

<style>
  /* Sound settings' choice rows (SoundSettings.svelte), with the menus' tick in place of its preview button */
  .levels { display: flex; flex-direction: column; gap: 8px; }
  .level { display: flex; align-items: center; gap: 8px; padding-right: 14px; background: var(--bg-2); border: 1px solid transparent; border-radius: var(--radius-lg); }
  .level.selected { border-color: var(--focus-ring); }
  .level :global(svg) { flex-shrink: 0; color: var(--accent-text); }
  .pick { flex: 1; display: flex; flex-direction: column; gap: 2px; padding: 12px 14px; border-radius: var(--radius-lg); cursor: pointer; }
  .level.selected .pick strong { color: var(--accent-text); }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
