<script lang="ts">
  import Icon from './Icon.svelte'
  import { api } from '../lib/tauri'
  import { saveFlow, submitFlow, type ReportState } from '../lib/report'

  /** `saveOnly`: not signed in (login screen) — the report can only be saved to a file. */
  let { onClose, saveOnly = false, serverUrl = null }: { onClose: () => void; saveOnly?: boolean; serverUrl?: string | null } = $props()

  let text = $state('')
  let st = $state<ReportState>({ step: 'edit', message: '' })
  const busy = $derived(st.step === 'sending' || st.step === 'saving')
  const done = $derived(st.step === 'sent' || st.step === 'saved')

  async function send(e: SubmitEvent) {
    e.preventDefault()
    if (saveOnly) return save()
    st = await submitFlow(text, { submit: api.submitReport, onStep: (s) => (st = s) })
    if (st.step === 'sent') setTimeout(onClose, 1500)
  }

  async function save() {
    st = { step: 'saving', message: '' }
    st = await saveFlow(text, { save: (t) => api.saveReport(t, serverUrl) })
  }
</script>

<div class="overlay modal-overlay">
  <div class="backdrop modal-backdrop" role="presentation" onclick={onClose}></div>
  <div class="modal" role="dialog" aria-label="Report a bug"><form onsubmit={send}>
    <div class="top">
      <h1 class="modal-title">Report a bug</h1>
      <button type="button" class="icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" /></button>
    </div>
    <textarea bind:value={text} maxlength="4000" rows="6" required placeholder="What happened? What were you doing?" disabled={busy || done}></textarea>
    <p class="note">Includes your app version, system info and recent app log.</p>
    {#if st.message}<p class:ok={done} class:err={st.step === 'failed'} role="status">{st.message}</p>{/if}
    <div class="actions">
      {#if st.step === 'failed' && !saveOnly}
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
  textarea { resize: vertical; min-height: 120px; padding: 10px 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-1); font: inherit; font-size: 14px; color: var(--text); }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .ok { margin: 0; font-size: 13px; color: var(--ok); word-break: break-all; }
  .err { margin: 0; font-size: 13px; color: var(--warn); }
  .actions { display: flex; justify-content: flex-end; }
</style>
