<script lang="ts">
  // "Send crash reports automatically" is off and last session crashed: its report was saved on
  // this computer, not sent. Ask once: Send, or Don't send (deleted either way once it's done).
  import { api, errorText } from '../lib/tauri'
  import { ui } from '../lib/store.svelte'

  let busy = $state(false)
  let error = $state('')
  let done = $state('')

  const what = $derived(ui.heldReport === 'unexpected-exit' ? 'Pulse closed unexpectedly last time.' : 'Pulse crashed last time.')

  async function send() {
    busy = true
    error = ''
    try {
      await api.sendHeldReport()
      ui.heldReport = null
      done = 'Crash report sent. Thanks.'
      setTimeout(() => (done = ''), 4000)
    } catch (e) {
      error = errorText(e) // still held: they can try again or not send it
    } finally {
      busy = false
    }
  }

  async function discard() {
    busy = true
    error = ''
    try {
      await api.discardHeldReport()
    } catch {
      /* removing a local file: nothing useful to show */
    } finally {
      ui.heldReport = null
      busy = false
    }
  }
</script>

{#if ui.heldReport}
  <span class="prompt" role="status">{what} Send the crash report (with the app log)?
    <button class="link" disabled={busy} onclick={send}>Send</button>
    <button class="link" disabled={busy} onclick={discard}>Don't send</button>
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </span>
{:else if done}
  <span class="prompt" role="status">{done}</span>
{/if}

<style>
  .prompt { font-size: 12px; font-weight: 600; color: #e8b04a; padding: 4px 10px; border-radius: 12px; background: rgba(232, 176, 74, .12); margin-right: 6px; }
  button.link { border: 0; background: none; color: inherit; font-size: inherit; font-weight: 600; text-decoration: underline; padding: 0 2px; }
  button:disabled { opacity: .6; }
  .err { display: block; margin-top: 2px; font-weight: 500; color: #f2616b; }
</style>
