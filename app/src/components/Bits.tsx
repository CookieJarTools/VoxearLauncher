import type { Progress, ReleaseChannel } from '../api'
import { bytes } from '../lib/format'

export function ChannelBadge({ channel }: { channel: ReleaseChannel }) {
  return <span className={`channel channel-${channel}`}>{channel}</span>
}

export function Switch({ on }: { on: boolean }) {
  return <span className={`switch ${on ? 'on' : ''}`} aria-hidden />
}

/** A progress bar for an install. Phases without a size show a moving bar instead. */
export function ProgressBar({ progress }: { progress: Progress | null }) {
  const fraction = progress && 'total' in progress && progress.total > 0 ? progress.done / progress.total : null
  return (
    <div className={`progress ${fraction === null ? 'indeterminate' : ''}`}>
      <div className="progress-fill" style={{ width: `${Math.round((fraction ?? 0) * 100)}%` }} />
    </div>
  )
}

export function phaseText(progress: Progress | null, queued: boolean): string {
  if (queued) return 'Waiting'
  if (!progress) return 'Starting'
  switch (progress.phase) {
    case 'downloading':
      return `Downloading · ${bytes(progress.done)} of ${bytes(progress.total)}`
    case 'verifying':
      return 'Checking signature'
    case 'unpacking':
      return `Unpacking · ${progress.total ? Math.round((progress.done / progress.total) * 100) : 0}%`
    case 'finishing':
      return 'Finishing'
  }
}
