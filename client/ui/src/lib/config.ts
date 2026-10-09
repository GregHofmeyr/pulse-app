/** Server address the login screen starts with: baked in by release builds, local otherwise. */
export function defaultServer(env: { VITE_DEFAULT_SERVER?: string }): string {
  return env.VITE_DEFAULT_SERVER?.trim() || 'http://127.0.0.1:7890'
}
