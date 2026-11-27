<script lang="ts">
  // The Typing text section's right panel (#727): "<name> is [ … ]" on one row, tap-to-use suggestions that wrap, a
  // live "<name> is …" line, and the problem (if any) under the input. Only what follows "is" is yours to write
  // (#652). It edits the editor's draft; nothing saves until Save.
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { app } from '../../lib/store.svelte'
  import { TYPING_SUGGESTIONS, TYPING_TEXT_MAX, typingTextProblem } from '../../lib/typingLine'

  let { state: st }: { state: ProfileEditorState } = $props()

  const name = $derived(st.draft.displayName.trim() || app.state.me?.username || '')
  const problem = $derived(typingTextProblem(st.draft.typingText))
  const text = $derived(st.draft.typingText.trim())
</script>

<section class="typing" aria-label="Typing text">
  <h2>Typing text</h2>
  <p class="hint">What shows while you type, in every server and DM. Blank is the plain “is typing…”.</p>
  <div class="typing-row">
    <span class="typing-who name-clip">{name}</span><span class="typing-is">is</span>
    <input class="input text" type="text" aria-label="Typing text" aria-invalid={!!problem} placeholder="typing…" bind:value={st.draft.typingText} />
  </div>
  {#if problem}<p class="error" role="alert">{problem}</p>{/if}
  <div class="chips" role="group" aria-label="Suggestions">
    {#each TYPING_SUGGESTIONS as t (t)}
      <button type="button" class="toggle-chip" aria-pressed={text === t} onclick={() => (st.draft.typingText = t)}>{t}</button>
    {/each}
  </div>
  <p class="typing-preview"><strong class="name-clip">{name}</strong> is {text || 'typing…'}</p>
  <p class="count" aria-hidden="true">{[...st.draft.typingText].length}/{TYPING_TEXT_MAX}</p>
</section>

<style>
  .typing { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  h2 { margin: 0; font-size: 15px; font-weight: 600; }
  .hint, .count { margin: 0; font-size: 12px; color: var(--text-3); overflow-wrap: anywhere; }
  .count { text-align: right; }
  .typing-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .typing-who { flex: 0 1 auto; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .typing-is { flex: none; color: var(--text-2); }
  .typing-row .input { flex: 1 1 0; min-width: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .chips :global(.toggle-chip) { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .typing-preview { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; color: var(--text-2); }
  .typing-preview strong { color: var(--text); }
  .error { margin: 0; min-width: 0; overflow-wrap: anywhere; color: var(--danger-text); font-size: 13px; }
</style>
