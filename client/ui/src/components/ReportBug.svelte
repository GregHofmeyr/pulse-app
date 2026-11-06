<script lang="ts">
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import { modal } from '../lib/actions'
  import { api } from '../lib/tauri'
  import { saveFlow, submitFlow, type ReportState } from '../lib/report'

  /** `saveOnly`: not signed in (login screen) — the report can only be saved to a file. */
  let { onClose, saveOnly = false, serverUrl = null }: { onClose: () => void; saveOnly?: boolean; serverUrl?: string | null } = $props()

  // #218: Bug or Idea. An idea is just the words (no log or diagnostics) and has to be sent, so the login screen's
  // save-only dialog stays a bug report.
  type Kind = 'bug' | 'idea'
  const KINDS: { value: Kind; label: string }[] = [{ value: 'bug', label: 'Bug' }, { value: 'idea', label: 'Idea' }]
  let kind = $state<Kind>('bug')
  const idea = $derived(kind === 'idea')
  const title = $derived(idea ? 'Suggest an idea' : 'Report a bug')

  let text = $state('')
  let st = $state<ReportState>({ step: 'edit', message: '' })
  const busy = $derived(st.step === 'sending' || st.step === 'saving')
  const done = $derived(st.step === 'sent' || st.step === 'saved')

  function setKind(k: Kind) {
    if (busy || done) return
    kind = k
    if (st.step === 'failed') st = { step: 'edit', message: '' }
  }

  async function send(e: SubmitEvent) {
    e.preventDefault()
    if (saveOnly) return save()
    st = idea
      ? await submitFlow(text, { submit: api.submitIdea, onStep: (s) => (st = s), noun: 'Idea' })
      : await submitFlow(text, { submit: api.submitReport, onStep: (s) => (st = s) })
    if (st.step === 'sent') setTimeout(onClose, 1500)
  }

  async function save() {
    st = { step: 'saving', message: '' }
    st = await saveFlow(text, { save: (t) => api.saveReport(t, serverUrl) })
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={onClose}><form onsubmit={send}>
    <div class="top">
      <h1 class="modal-title">{title}</h1>
      <button type="button" class="icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" size={16} /></button>
    </div>
    {#if !saveOnly}
      <div class="kind" class:locked={busy || done}><Segmented label="Report type" options={KINDS} value={kind} onchange={setKind} /></div>
    {/if}
    <textarea class="input" class:idea bind:value={text} maxlength="4000" rows={idea ? 12 : 6} required
      placeholder={idea ? 'What would make Pulse better for you?' : 'What happened? What were you doing?'} disabled={busy || done}></textarea>
    <p class="note">{idea ? 'No logs or system info are attached, just what you write.' : 'Includes your app version, system info and recent app log.'}</p>
    {#if st.message}<p class:ok={done} class:err={st.step === 'failed'} role="status">{st.message}</p>{/if}
    <div class="actions">
      {#if st.step === 'failed' && !saveOnly && !idea}
        <button type="button" class="btn btn-lg btn-primary" onclick={save}>Save report</button>
      {:else if done}
        <button type="button" class="btn btn-lg btn-primary" onclick={onClose}>Done</button>
      {:else}
        <button class="btn btn-lg btn-primary" disabled={busy || !text.trim()}>{busy ? '…' : saveOnly ? 'Save report' : 'Send'}</button>
      {/if}
    </div>
  </form></div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(460px, 100%); }
  form { display: contents; }
  .top { display: flex; align-items: center; justify-content: space-between; }
  .kind { display: flex; }
  /* mid-send or sent: the kind can't change under it */
  .kind.locked { pointer-events: none; opacity: .6; }
  textarea { resize: vertical; min-height: 120px; padding: 10px 12px; }
  textarea.idea { min-height: 220px; }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .ok { margin: 0; font-size: 13px; color: var(--ok-text); word-break: break-all; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; }
</style>
