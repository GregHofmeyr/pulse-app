import { mount } from 'svelte'
// Bundled locally: the app makes no third-party requests (CSP blocks them anyway).
import './fonts.css'
import './nameFonts.css'
import './app.css'
import App from './App.svelte'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
