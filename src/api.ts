import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { LiveSession, UsageSummary } from './types'

export const getLiveSessions = () => invoke<LiveSession[]>('live_sessions')
export const getUsageSummary = () => invoke<UsageSummary>('usage_summary')
export const reindex = () => invoke('reindex')

export const onSessionsChanged = (cb: (s: LiveSession[]) => void) =>
  listen<LiveSession[]>('sessions-changed', (e) => cb(e.payload))
