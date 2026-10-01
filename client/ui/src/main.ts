import { mount } from 'svelte'
// Bundled locally: the app makes no third-party requests (CSP blocks them anyway).
import '@fontsource/onest/400.css'
import '@fontsource/onest/500.css'
import '@fontsource/onest/600.css'
import '@fontsource/onest/700.css'
import './app.css'
import App from './App.svelte'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
