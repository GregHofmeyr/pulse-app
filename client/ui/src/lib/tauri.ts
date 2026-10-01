// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Server } from './protocol/Server'

export const api = {
  login: (serverUrl: string, username: string, password: string) =>
    invoke<User>('login', { serverUrl, username, password }),
  register: (serverUrl: string, inviteCode: string, username: string, password: string) =>
    invoke<User>('register', { serverUrl, inviteCode, username, password }),
  restoreSession: () => invoke<User | null>('restore_session'),
  logout: () => invoke<void>('logout'),
  listServers: () => invoke<Server[]>('list_servers'),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong'
}
