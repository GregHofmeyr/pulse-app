<script lang="ts">
  import ConnText from '../components/ConnText.svelte'
</script>

<!-- Signed in, but the server turned this connection away: keep retrying; the app opens on Ready. -->
<div class="waiting" role="status"><p><ConnText /></p></div>

<style>
  .waiting { height: 100%; display: grid; place-items: center; background: var(--bg-0); color: var(--text-1, inherit); text-align: center; padding: 24px; }
</style>
