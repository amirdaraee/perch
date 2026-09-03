import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { getLiveSessions, getUsageSummary, onSessionsChanged } from './api'
import type { LiveSession, UsageSummary } from './types'
import { cost, elapsed, tokens } from './format'

function StatusDot({ s }: { s: LiveSession }) {
  // 'idle' is a real status Claude Code publishes — dim green, not the same as working.
  const cls =
    s.status.kind === 'waiting' ? 'dot y'
    : s.kind === 'bg' ? 'dot w'
    : s.status.kind === 'idle' ? 'dot gi'
    : 'dot g'
  return <span className={cls} />
}

function SessionRow({ s, now }: { s: LiveSession; now: number }) {
  const waiting = s.status.kind === 'waiting' ? s.status : null
  // A just-started session can have no statusUpdatedAt at all; 0 means "absent", not "epoch".
  const rawSince = waiting ? waiting.sinceMs : s.statusUpdatedAt
  const since = rawSince > 0 ? rawSince : null
  return (
    <div className="srow">
      <StatusDot s={s} />
      <span className="sname">
        <b>{s.name || s.sessionId.slice(0, 8)}</b>
        <i>{waiting ? `waiting · ${waiting.reason ?? 'unknown'}` : 'working'}</i>
      </span>
      <span className="stime">{elapsed(since === null ? null : now - since)}</span>
    </div>
  )
}

export default function App() {
  const [sessions, setSessions] = useState<LiveSession[]>([])
  const [usage, setUsage] = useState<UsageSummary | null>(null)
  const [now, setNow] = useState(Date.now())

  useEffect(() => {
    getLiveSessions().then(setSessions).catch(() => setSessions([]))
    getUsageSummary().then(setUsage).catch(() => setUsage(null))
    const un = onSessionsChanged(setSessions)
    const tick = setInterval(() => setNow(Date.now()), 1000)
    return () => {
      un.then((f) => f())
      clearInterval(tick)
    }
  }, [])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        void getCurrentWindow().hide()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  const waiting = sessions.filter((s) => s.status.kind === 'waiting')

  return (
    <div className="popover">
      <div className="stats">
        <div className="stat">
          <div className="lbl">Window</div>
          <div className="num">{usage ? tokens(usage.window.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">Week</div>
          <div className="num">{usage ? tokens(usage.week.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">24h</div>
          <div className="num">{usage ? cost(usage.today.costUsd) : '—'}</div>
          {usage && <div className="est">est</div>}
        </div>
      </div>

      {waiting.length > 0 && (
        <div className="banner">
          {waiting.length === 1
            ? `1 session is waiting on you`
            : `${waiting.length} sessions are waiting on you`}
        </div>
      )}

      <div className="lbl section">Live · {sessions.length}</div>
      {sessions.length === 0 && <div className="empty">No sessions running</div>}
      {sessions.map((s) => (
        <SessionRow key={s.sessionId} s={s} now={now} />
      ))}
    </div>
  )
}
