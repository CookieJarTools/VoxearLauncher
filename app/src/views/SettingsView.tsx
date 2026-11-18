import { useEffect, useState } from 'react'
import { api, failure, type CustomReleases, type Progress, type Settings } from '../api'
import { phaseText, ProgressBar, Switch } from '../components/Bits'
import { Icon } from '../components/Icon'
import { ago, bytes, pretty } from '../lib/format'
import { useStore } from '../store'
import { reveal } from './projectActions'

export function SettingsView() {
  const info = useStore((s) => s.info)
  const toast = useStore((s) => s.toast)
  const [saved, setSaved] = useState<Settings | null>(null)
  const [keys, setKeys] = useState('')
  const [location, setLocation] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    api.getSettings().then(
      (settings) => {
        setSaved(settings)
        setKeys((settings.trusted_keys ?? []).join('\n'))
        setLocation(settings.project_location ?? '')
      },
      (error) => toast('error', failure(error).message)
    )
  }, [toast])

  const next: Settings = {
    trusted_keys: keys
      .split('\n')
      .map((key) => key.trim())
      .filter(Boolean),
    project_location: location.trim() || undefined
  }
  const dirty =
    saved !== null &&
    ((saved.trusted_keys ?? []).join('\n') !== (next.trusted_keys ?? []).join('\n') ||
      (saved.project_location ?? '') !== (next.project_location ?? ''))

  const save = async () => {
    setSaving(true)
    try {
      // Keeps what this form doesn't edit, such as the launcher channel and custom releases.
      const merged = { ...(await api.getSettings()), ...next }
      await api.saveSettings(merged)
      setSaved(merged)
      toast('ok', 'Settings saved')
      const store = useStore.getState()
      void store.loadReleases()
      void api.appInfo().then((info) => useStore.setState({ info }))
    } catch (error) {
      toast('error', failure(error).message)
    } finally {
      setSaving(false)
    }
  }

  const browseLocation = async () => {
    const folder = await api.pickFolder('Where should new projects go?', location || info?.defaultLocation)
    if (folder) setLocation(folder)
  }

  return (
    <section className="view">
      <div className="view-header">
        <h1 className="view-title">Settings</h1>
        <button className="btn btn-primary" onClick={() => void save()} disabled={!dirty || saving}>
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
      <div className="view-scroll">
        <div className="settings">
          <div className="settings-section">
            <h2>Engine releases</h2>
            <ReleaseLists />
            <label className="field">
              <span className="label">Extra trusted keys</span>
              <textarea
                className="input"
                value={keys}
                placeholder="One minisign public key per line"
                onChange={(e) => setKeys(e.target.value)}
                spellCheck={false}
              />
              <span className="hint">Trusted for every release list, and for archives installed from a file.</span>
            </label>
          </div>

          <div className="settings-section">
            <h2>Projects</h2>
            <div className="field">
              <span className="label">New projects go in</span>
              <div className="input-row">
                <input
                  className="input"
                  value={location}
                  placeholder={info?.defaultLocation}
                  onChange={(e) => setLocation(e.target.value)}
                  spellCheck={false}
                />
                <button className="btn" onClick={() => void browseLocation()}>
                  Browse
                </button>
              </div>
              <span className="hint">The last folder you created a project in is remembered here.</span>
            </div>
          </div>

          <WhiskSection />

          <LauncherUpdates />

          <div className="settings-section">
            <h2>About</h2>
            <dl className="facts">
              <dt>Launcher</dt>
              <dd>{info?.version}</dd>
              <span />
              <dt>Platform</dt>
              <dd>{info?.platform}</dd>
              <span />
              {info?.appMenu.kind === 'installed' && (
                <>
                  <dt>Installed at</dt>
                  <dd title={info.appMenu.path}>{pretty(info.appMenu.path, info.userHome)}</dd>
                  <span />
                </>
              )}
              <dt>Launcher folder</dt>
              <dd title={info?.home}>{info ? pretty(info.home, info.userHome) : ''}</dd>
              <button className="btn btn-sm btn-ghost" onClick={() => info && void reveal(info.home)}>
                <Icon name="folder" size={14} />
                Show
              </button>
              <dt>Settings file</dt>
              <dd title={info?.settingsFile}>{info ? pretty(info.settingsFile, info.userHome) : ''}</dd>
              <span />
            </dl>
            <span className="hint">Everything here can also be done from a terminal with voxup.</span>
          </div>
        </div>
      </div>
    </section>
  )
}

/** Where engine releases come from: the official ones, always listed; nightlies; and custom releases, such as a fork's.
 * Each change saves at once, as the launcher's channel does. */
function ReleaseLists() {
  const toast = useStore((s) => s.toast)
  const installs = useStore((s) => s.installs)
  const home = useStore((s) => s.info?.userHome)
  const [settings, setSettings] = useState<Settings | null>(null)
  const [adding, setAdding] = useState(false)
  const [address, setAddress] = useState('')
  const [key, setKey] = useState('')
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getSettings().then(setSettings, (error) => toast('error', failure(error).message))
  }, [toast])

  // Unset, nightlies show once one is installed.
  const nightlies = settings?.nightlies ?? installs.some((install) => install.channel === 'nightly')
  const lists = settings?.custom_releases ?? []

  const reload = async () => {
    setSettings(await api.getSettings())
    void useStore.getState().loadReleases()
  }

  const setNightlies = async (on: boolean) => {
    try {
      await api.saveSettings({ ...(await api.getSettings()), nightlies: on })
      await reload()
    } catch (error) {
      toast('error', failure(error).message)
    }
  }

  const close = () => {
    setAdding(false)
    setAddress('')
    setKey('')
    setName('')
    setError(null)
  }

  const add = async () => {
    setBusy(true)
    setError(null)
    try {
      const added = await api.addCustomReleases(address.trim(), name.trim() || null, key.trim() || null)
      const found = added.releases === null ? 'nothing published yet' : `${added.releases} ${added.releases === 1 ? 'release' : 'releases'}`
      toast('ok', `Added ${added.name} · ${found}`)
      close()
      await reload()
    } catch (error) {
      setError(failure(error).message)
    } finally {
      setBusy(false)
    }
  }

  const remove = async (list: CustomReleases) => {
    const ok = await useStore.getState().confirm({
      title: `Remove ${list.name}?`,
      message: 'Its releases stop showing in the list. Engines installed from it stay.',
      confirm: 'Remove',
      danger: true
    })
    if (!ok) return
    try {
      await api.removeCustomReleases(list.name)
      await reload()
      toast('ok', `Removed ${list.name}`)
    } catch (error) {
      toast('error', failure(error).message)
    }
  }

  const browse = async () => {
    const file = await api.pickFile('Choose a releases.toml', 'Release manifest', ['toml'])
    if (file) setAddress(file)
  }

  return (
    <>
      <button className="switch-row" style={{ margin: '-6px -10px' }} onClick={() => void setNightlies(!nightlies)} disabled={!settings}>
        <span className="text">
          Nightlies
          <small>Builds of the newest engine, made each night</small>
        </span>
        <Switch on={nightlies} />
      </button>
      <div className="field">
        <span className="label">Custom engine releases</span>
        {lists.map((list) => (
          <div key={list.name} className="input-row" style={{ alignItems: 'center' }}>
            <span className="release-version">{list.name}</span>
            <span className="hint" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={list.url}>
              {shortAddress(list.url, home)}
            </span>
            {list.key && <span className="tag">Own key</span>}
            <button className="btn btn-sm btn-ghost" onClick={() => void remove(list)}>
              Remove
            </button>
          </div>
        ))}
        {adding ? (
          <div className="field" style={{ gap: 10 }}>
            <label className="field">
              <span className="label">Address</span>
              <div className="input-row">
                <input
                  className="input"
                  value={address}
                  placeholder="owner/repo, or https://…/releases.toml"
                  onChange={(e) => setAddress(e.target.value)}
                  spellCheck={false}
                  autoFocus
                />
                <button className="btn" onClick={() => void browse()}>
                  Browse
                </button>
              </div>
            </label>
            <label className="field">
              <span className="label">Public key</span>
              <input className="input" value={key} placeholder="RW…" onChange={(e) => setKey(e.target.value)} spellCheck={false} />
            </label>
            <label className="field">
              <span className="label">Name</span>
              <input className="input" value={name} placeholder="From the address" onChange={(e) => setName(e.target.value)} spellCheck={false} />
            </label>
            {error && <span className="error-text">{error}</span>}
            <div className="input-row">
              <button className="btn btn-primary btn-sm" onClick={() => void add()} disabled={busy || !address.trim()}>
                {busy ? 'Checking…' : 'Add'}
              </button>
              <button className="btn btn-sm btn-ghost" onClick={close} disabled={busy}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <div>
            <button className="btn btn-sm" onClick={() => setAdding(true)} disabled={!settings}>
              <Icon name="plus" size={14} />
              Add
            </button>
          </div>
        )}
        <span className="hint">Someone else's releases, such as a fork of the engine, listed beside the official ones.</span>
      </div>
    </>
  )
}

/** A release list's address, short: a GitHub Pages site as owner/repo, a site without its manifest's path. */
function shortAddress(url: string, home: string | undefined): string {
  const pages = url.match(/^https:\/\/([^./]+)\.github\.io\/([^/]+)\/engine\/releases\.toml$/)
  if (pages) return `${pages[1]}/${pages[2]}`
  if (url.startsWith('https://')) return url.slice('https://'.length).replace(/\/engine\/releases\.toml$/, '')
  return pretty(url, home)
}

/** The launcher's own updates: its channel, a check, and what the last check found. */
function LauncherUpdates() {
  const update = useStore((s) => s.launcherUpdate)
  const check = useStore((s) => s.checkLauncherUpdate)
  const install = useStore((s) => s.installLauncherUpdate)
  const toast = useStore((s) => s.toast)
  const [channel, setChannel] = useState<'stable' | 'beta'>('stable')

  useEffect(() => {
    api.getSettings().then((settings) => setChannel(settings.launcher_channel ?? 'stable'), () => {})
  }, [])

  // The channel is a preference, so it saves at once, and checks again.
  const choose = async (next: 'stable' | 'beta') => {
    setChannel(next)
    try {
      const settings = await api.getSettings()
      await api.saveSettings({ ...settings, launcher_channel: next })
      void check()
    } catch (error) {
      toast('error', failure(error).message)
    }
  }

  let status: string
  if (update.state === 'checking') status = 'Checking…'
  else if (update.state === 'downloading') status = `Downloading ${update.version}${update.total ? ` · ${bytes(update.done)} of ${bytes(update.total)}` : ''}`
  else if (update.state === 'ready') status = `${update.version} is installed and starts when you restart the launcher.`
  else if (update.error) status = update.error
  else if (!update.status) status = 'Not checked yet.'
  else if (update.status.kind === 'notSetUp') status = update.status.reason
  else if (update.status.kind === 'nothingPublished') status = 'No launcher updates are published yet.'
  else if (update.status.kind === 'upToDate') status = `${update.status.version} is the newest ${channel} version.`
  else status = `${update.status.version} is out.`
  const checked = update.state === 'idle' && update.checkedAt ? ` Checked ${ago(new Date(update.checkedAt).toISOString())}.` : ''

  return (
    <div className="settings-section">
      <h2>Launcher updates</h2>
      <div className="field">
        <span className="label">Channel</span>
        <div className="segmented" style={{ justifySelf: 'start' }}>
          {(['stable', 'beta'] as const).map((id) => (
            <button key={id} className={channel === id ? 'on' : ''} onClick={() => void choose(id)}>
              {id === 'stable' ? 'Stable' : 'Beta'}
            </button>
          ))}
        </div>
        <span className="hint">Engine versions have their own channels; this is only for the launcher.</span>
      </div>
      <div className="input-row" style={{ alignItems: 'center' }}>
        <span className={update.state === 'idle' && update.error ? 'error-text' : 'hint'} style={{ flex: 1 }}>
          {status}
          {checked}
        </span>
        {update.state === 'idle' && update.status?.kind === 'available' ? (
          <button className="btn btn-primary btn-sm" onClick={() => void install()}>
            Update to {update.status.version}
          </button>
        ) : (
          <button className="btn btn-sm" onClick={() => void check()} disabled={update.state !== 'idle'}>
            Check now
          </button>
        )}
      </div>
      {update.state === 'downloading' && (
        <ProgressBar progress={update.total ? { phase: 'downloading', done: update.done, total: update.total } : null} />
      )}
    </div>
  )
}

/** Whisk, the code editor Voxear opens scripts in: install it, open it, or remove the copy the launcher installed. */
function WhiskSection() {
  const info = useStore((s) => s.info)
  const whisk = useStore((s) => s.whisk)
  const toast = useStore((s) => s.toast)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [busy, setBusy] = useState(false)

  const run = async (work: () => Promise<void>) => {
    setBusy(true)
    try {
      await work()
    } catch (error) {
      toast('error', failure(error).message)
    } finally {
      setBusy(false)
      setProgress(null)
    }
  }
  const install = () =>
    run(async () => {
      const found = await api.installWhisk(setProgress)
      useStore.setState({ whisk: found })
      toast('ok', 'Whisk is installed')
    })
  const remove = () =>
    run(async () => {
      useStore.setState({ whisk: await api.removeWhisk() })
      toast('ok', 'Removed Whisk')
    })
  const open = () => run(() => api.openWhisk(null))

  let status = 'Not installed.'
  if (busy && progress) status = phaseText(progress, false)
  else if (whisk?.installed && whisk.path) status = info ? pretty(whisk.path, info.userHome) : whisk.path

  return (
    <div className="settings-section">
      <h2>Code editor</h2>
      <div className="field">
        <span className="label">Whisk</span>
        <div className="input-row" style={{ alignItems: 'center' }}>
          <span className="hint" style={{ flex: 1 }} title={whisk?.path ?? undefined}>
            {status}
          </span>
          {whisk?.installed ? (
            <>
              <button className="btn btn-sm" onClick={() => void open()} disabled={busy}>
                Open
              </button>
              {whisk.byLauncher && (
                <button className="btn btn-sm btn-ghost" onClick={() => void remove()} disabled={busy}>
                  Remove
                </button>
              )}
            </>
          ) : (
            <button className="btn btn-primary btn-sm" onClick={() => void install()} disabled={busy}>
              {busy ? 'Installing…' : 'Install'}
            </button>
          )}
        </div>
        {busy && progress && <ProgressBar progress={progress} />}
        <span className="hint">The editor opens scripts in Whisk. It keeps itself up to date once installed.</span>
      </div>
    </div>
  )
}
