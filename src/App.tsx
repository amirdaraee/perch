import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { getLiveSessions, getUsageSummary, onSessionsChanged, reindex } from './api'
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
        <i>
          {waiting ? `waiting · ${waiting.reason ?? 'unknown'}`
          : s.status.kind === 'idle' ? 'idle'
          : 'working'}
        </i>
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
    const loadUsage = () => getUsageSummary().then(setUsage).catch(() => setUsage(null))

    getLiveSessions().then(setSessions).catch(() => setSessions([]))
    // Nothing else populates the index, so without this the popover would show
    // an empty database forever. Cold ~0.25 s, warm ~0.01 s; a failed reindex
    // must still not stop us reading whatever is already indexed.
    void reindex().catch(() => {}).then(loadUsage)
    // The watcher emits at least every 5 s, so usage stays fresh at no extra
    // polling cost — and the popover is not remounted on hide/show.
    const un = onSessionsChanged((s) => {
      setSessions(s)
      void loadUsage()
    })
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
  // An index with no turns in it reports zeros, which would read as "you spent
  // nothing". Show em-dashes until there is something real to show.
  const stats = usage && usage.hasData ? usage : null

  return (
    <div className="popover">
      <div className="stats">
        <div className="stat">
          <div className="lbl">Window</div>
          <div className="num">{stats ? tokens(stats.window.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">Week</div>
          <div className="num">{stats ? tokens(stats.week.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">24h</div>
          <div className="num">{stats ? cost(stats.today.costUsd) : '—'}</div>
          {stats && <div className="est">est</div>}
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
