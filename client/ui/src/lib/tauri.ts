// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Server } from './protocol/Server'
import type { Channel } from './protocol/Channel'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'

export const api = {
  login: (serverUrl: string, username: string, password: string) =>
    invoke<User>('login', { serverUrl, username, password }),
  register: (serverUrl: string, inviteCode: string, username: string, password: string) =>
    invoke<User>('register', { serverUrl, inviteCode, username, password }),
  restoreSession: () => invoke<User | null>('restore_session'),
  logout: () => invoke<void>('logout'),
  reconnectNow: () => invoke<void>('gateway_reconnect_now'),
  listServers: () => invoke<Server[]>('list_servers'),
  joinServer: (serverId: string) => invoke<void>('join_server', { serverId }),
  listChannels: (serverId: string) => invoke<Channel[]>('list_channels', { serverId }),
  listMembers: (serverId: string) => invoke<Member[]>('list_members', { serverId }),
  listMessages: (channelId: string, before?: string) =>
    invoke<Message[]>('list_messages', { channelId, before: before ?? null }),
  sendMessage: (channelId: string, content: string, replyToId?: string, nonce?: string) =>
    invoke<Message>('send_message', { channelId, content, replyToId: replyToId ?? null, nonce: nonce ?? null }),
  editMessage: (messageId: string, content: string) => invoke<Message>('edit_message', { messageId, content }),
  deleteMessage: (messageId: string) => invoke<void>('delete_message', { messageId }),
  sendTyping: (channelId: string) => invoke<void>('send_typing', { channelId }),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong'
}
