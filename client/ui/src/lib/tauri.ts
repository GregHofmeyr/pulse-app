// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Server } from './protocol/Server'
import type { Channel } from './protocol/Channel'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { MediaInfo } from './protocol/MediaInfo'
import type { CustomEmoji } from './protocol/CustomEmoji'

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
  /** The raw file goes as the request body (a JSON number array would be slow at 2 MB). */
  uploadEmojiMedia: (bytes: Uint8Array) => invoke<MediaInfo>('upload_emoji_media', bytes),
  createEmoji: (serverId: string, name: string, mediaHash: string) =>
    invoke<CustomEmoji>('create_emoji', { serverId, name, mediaHash }),
  renameEmoji: (id: string, name: string) => invoke<CustomEmoji>('rename_emoji', { id, name }),
  deleteEmoji: (id: string) => invoke<void>('delete_emoji', { id }),
  /** `emoji`: a Unicode emoji or `custom:<id>`. */
  addReaction: (messageId: string, emoji: string) => invoke<void>('add_reaction', { messageId, emoji }),
  removeReaction: (messageId: string, emoji: string) => invoke<void>('remove_reaction', { messageId, emoji }),
  listReactors: (messageId: string, emoji: string) => invoke<string[]>('list_reactors', { messageId, emoji }),
  /** Returns the report's short id. */
  submitReport: (text: string) => invoke<string>('submit_report', { text }),
  /** Writes a .txt to Downloads; returns its path. `serverUrl` when not signed in. */
  saveReport: (text: string, serverUrl: string | null) => invoke<string>('save_report', { text, serverUrl }),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong'
}
