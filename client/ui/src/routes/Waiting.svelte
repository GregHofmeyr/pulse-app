<script lang="ts">
  import ConnText from '../components/ConnText.svelte'

  let { onSignOut }: { onSignOut: () => void } = $props()
</script>

<!-- Signed in, but the server turned this connection away: keep retrying; the app opens on Ready.
     Signing out is the way off this screen (another account, or give up on this device). -->
<div class="waiting">
  <p role="status"><ConnText /></p>
  <button class="btn btn-secondary" onclick={onSignOut}>Sign out</button>
</div>

<style>
  .waiting { height: 100%; display: grid; place-content: center; justify-items: center; gap: 16px; background: var(--bg-0); color: var(--text); text-align: center; padding: 24px; }
</style>
