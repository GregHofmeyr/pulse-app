// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Server } from './protocol/Server'
import type { Channel } from './protocol/Channel'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { StreamInfo } from './protocol/StreamInfo'
import type { StreamSettings } from './protocol/StreamSettings'
import type { VoiceTokenResponse } from './protocol/VoiceTokenResponse'

export const api = {
  createDm: (userIds: string[]) => invoke<Channel>('create_dm', { userIds }),
  addMembers: (channelId: string, userIds: string[]) => invoke<void>('add_members', { channelId, userIds }),
  removeMember: (channelId: string, userId: string) => invoke<void>('remove_member', { channelId, userId }),
  renameChannel: (channelId: string, name: string | null) => invoke<Channel>('rename_channel', { channelId, name }),
  markRead: (channelId: string, messageId: string) => invoke<void>('mark_read', { channelId, messageId }),
  setMute: (targetKind: 'server' | 'channel', targetId: string, until: string | null) => invoke<void>('set_mute', { targetKind, targetId, until }),
  clearMute: (targetKind: 'server' | 'channel', targetId: string) => invoke<void>('clear_mute', { targetKind, targetId }),
  closeConversation: (channelId: string) => invoke<void>('close_conversation', { channelId }),
  login: (serverUrl: string, username: string, password: string) =>
    invoke<User>('login', { serverUrl, username, password }),
  register: (serverUrl: string, inviteCode: string, username: string, password: string) =>
    invoke<User>('register', { serverUrl, inviteCode, username, password }),
  restoreSession: () => invoke<User | null>('restore_session'),
  logout: () => invoke<void>('logout'),
  probe: () => invoke<void>('gateway_probe'),
  listServers: () => invoke<Server[]>('list_servers'),
  createServer: (name: string) => invoke<Server>('create_server', { name }),
  createChannel: (serverId: string, kind: 'text' | 'voice', name: string) =>
    invoke<Channel>('create_channel', { serverId, kind, name }),
  joinServer: (serverId: string) => invoke<void>('join_server', { serverId }),
  listChannels: (serverId: string) => invoke<Channel[]>('list_channels', { serverId }),
  listMembers: (serverId: string) => invoke<Member[]>('list_members', { serverId }),
  listMessages: (channelId: string, before?: string) =>
    invoke<Message[]>('list_messages', { channelId, before: before ?? null }),
  /** Queued in the Rust outbox; the confirmed message arrives as a MessageCreated event with this nonce. */
  sendMessage: (channelId: string, content: string, replyToId: string | null, nonce: string) =>
    invoke<void>('send_message', { channelId, content, replyToId, nonce }),
  editMessage: (messageId: string, content: string) => invoke<Message>('edit_message', { messageId, content }),
  deleteMessage: (messageId: string) => invoke<void>('delete_message', { messageId }),
  sendTyping: (channelId: string) => invoke<void>('send_typing', { channelId }),
  /** Returns the report's short id. */
  submitReport: (text: string) => invoke<string>('submit_report', { text }),
  /** Writes a .txt to Downloads; returns its path. `serverUrl` when not signed in. */
  saveReport: (text: string, serverUrl: string | null) => invoke<string>('save_report', { text, serverUrl }),
}

/** Screen-share commands (the `#v` ticket, live/watch bookkeeping, previews, and the Rust `#a` audio side). */
export const streamCmd = {
  ticket: (channelId: string, role: 'v' | 'a') => invoke<VoiceTokenResponse>('stream_ticket', { channelId, role }),
  goLive: (channelId: string, settings: StreamSettings) => invoke<StreamInfo>('stream_go_live', { channelId, settings }),
  stop: (channelId: string) => invoke<void>('stream_stop', { channelId }),
  watch: (channelId: string, streamer: string) => invoke<void>('stream_watch', { channelId, streamer }),
  unwatch: (channelId: string, streamer: string) => invoke<void>('stream_unwatch', { channelId, streamer }),
  /** `webpBase64` without the `data:` prefix. */
  previewPut: (channelId: string, webpBase64: string) => invoke<void>('stream_preview_put', { channelId, webpBase64 }),
  /** A `data:` URL, or null when the streamer hasn't sent one yet. */
  previewGet: (channelId: string, userId: string) => invoke<string | null>('stream_preview_get', { channelId, userId }),
  /** One JSON object, no newlines; Rust prefixes `share: `. */
  log: (line: object) => invoke<void>('stream_log', { line: JSON.stringify(line) }).catch(() => {}),
  pickerWindow: (open: boolean) => invoke<void>('stream_picker_window', { open }),
  audioStart: (channelId: string, windowTitle: string | null) => invoke<void>('stream_audio_start', { channelId, windowTitle }),
  audioStop: () => invoke<void>('stream_audio_stop'),
  audioMute: (muted: boolean) => invoke<void>('stream_audio_mute', { muted }),
  audioWatch: (channelId: string, streamer: string, on: boolean) => invoke<void>('stream_audio_watch', { channelId, streamer, on }),
  setVolume: (userId: string, pct: number) => invoke<void>('set_stream_volume', { userId, pct }),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong'
}
