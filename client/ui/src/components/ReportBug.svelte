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

<div class="overlay">
  <div class="backdrop" role="presentation" onclick={onClose}></div>
  <div class="modal" role="dialog" aria-label="Report a bug"><form onsubmit={send}>
    <div class="top">
      <h1>Report a bug</h1>
      <button type="button" class="close" aria-label="Close" onclick={onClose}><Icon name="x" /></button>
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
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: var(--scrim); }
  .modal { position: relative; width: min(460px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; padding: 22px; border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line); display: flex; flex-direction: column; gap: 12px; }
  form { display: contents; }
  .top { display: flex; align-items: center; justify-content: space-between; }
  h1 { margin: 0; font-size: 18px; font-weight: 600; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  .close:hover { background: var(--bg-4); color: var(--text); }
  textarea { resize: vertical; min-height: 120px; padding: 10px 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-1); font: inherit; font-size: 14px; color: var(--text); }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .ok { margin: 0; font-size: 13px; color: var(--ok); word-break: break-all; }
  .err { margin: 0; font-size: 13px; color: var(--warn); }
  .actions { display: flex; justify-content: flex-end; }
</style>
