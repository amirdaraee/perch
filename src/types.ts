export type SessionStatus =
  | { kind: 'working' }
  | { kind: 'idle' }
  | { kind: 'waiting'; reason: string | null; sinceMs: number }
  | { kind: 'background' }
  | { kind: 'ended' }

export interface LiveSession {
  pid: number
  sessionId: string
  cwd: string
  name: string
  kind: string
  status: SessionStatus
  startedAt: number
  statusUpdatedAt: number
  ccVersion: string | null
  socketPath: string | null
}

export interface UsageSlice { tokens: number; costUsd: number }

export interface UsageSummary {
  window: UsageSlice
  week: UsageSlice
  today: UsageSlice
  source: 'estimated' | 'api'
  /** False when the index holds no turns yet: the zeros mean "nothing indexed", not "no spend". */
  hasData: boolean
}
