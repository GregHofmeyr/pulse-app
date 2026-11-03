import { mount } from 'svelte'
// Bundled locally: the app makes no third-party requests (CSP blocks them anyway).
import './fonts.css'
import './nameFonts.css'
import './app.css'
import App from './App.svelte'
import { applyTheme } from './lib/theme.svelte'

// The saved theme before the first paint (no flash of Pulse Dark on Light).
applyTheme()

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
