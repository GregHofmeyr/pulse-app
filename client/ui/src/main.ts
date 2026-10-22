import { mount } from 'svelte'
// Bundled locally: the app makes no third-party requests (CSP blocks them anyway).
import './fonts.css'
import './app.css'
import App from './App.svelte'

// THROWAWAY (spike S18): a pop-out window renders only its viewer, without App's session start-up.
const popout = import.meta.env.VITE_SPIKE === '1' ? new URLSearchParams(location.search).get('spike-popout') : null
if (import.meta.env.VITE_SPIKE === '1' && popout !== null) {
  const channel = new URLSearchParams(location.search).get('ch') ?? ''
  void import('./spike/SpikePopout.svelte').then(({ default: SpikePopout }) =>
    mount(SpikePopout, { target: document.getElementById('app')!, props: { streamer: popout, channel } }),
  )
} else {
  mount(App, { target: document.getElementById('app')! })
}
