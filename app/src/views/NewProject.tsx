import { useEffect, useMemo, useRef, useState } from 'react'
import { api, failure, type NewProjectForm, type Offer } from '../api'
import { Switch } from '../components/Bits'
import { Icon } from '../components/Icon'
import { Rich } from '../components/Rich'
import { TemplateArt } from '../components/TemplateArt'
import { joinPath, pretty } from '../lib/format'
import { useStore } from '../store'
import { openProject } from './projectActions'

/** The new-project flow. The form comes from what the chosen engine version's `vox new` offers. */
export function NewProject() {
  const { open, version: preset } = useStore((s) => s.newProject)
  if (!open) return null
  return <NewProjectDialog preset={preset} />
}

function NewProjectDialog({ preset }: { preset?: string }) {
  const close = useStore((s) => s.closeNewProject)
  const allInstalls = useStore((s) => s.installs)
  const installs = useMemo(() => allInstalls.filter((install) => !install.missing), [allInstalls])
  const info = useStore((s) => s.info)
  const usable = installs.length > 0

  const [version, setVersion] = useState(() => preset ?? (installs.find((i) => i.channel === 'stable') ?? installs[0])?.version ?? '')
  const [offer, setOffer] = useState<Offer | null>(null)
  const [offerError, setOfferError] = useState<string | null>(null)
  const [name, setName] = useState('My Game')
  const [location, setLocation] = useState(info?.defaultLocation ?? '')
  const [template, setTemplate] = useState('3d')
  const [setup, setSetup] = useState<string[]>([])
  const [ide, setIde] = useState('')
  const [git, setGit] = useState(false)
  const [modding, setModding] = useState(false)
  const [steamAppId, setSteamAppId] = useState('')
  const [extrasOpen, setExtrasOpen] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const [steps, setSteps] = useState<string[] | null>(null)
  const [createError, setCreateError] = useState<string | null>(null)
  const nameRef = useRef<HTMLInputElement>(null)

  // Ask the chosen engine version what it offers.
  useEffect(() => {
    if (!version) return
    let current = true
    setOffer(null)
    setOfferError(null)
    api.describeNew(version).then(
      (offer) => {
        if (!current) return
        setOffer(offer)
        const chosen = offer.templates.find((t) => t.id === template) ?? offer.templates.find((t) => t.id === '3d') ?? offer.templates[0]
        if (chosen) {
          setTemplate(chosen.id)
          setSetup(chosen.setup)
        }
      },
      (error) => current && setOfferError(failure(error).message)
    )
    return () => {
      current = false
    }
    // The current template only seeds the choice, so changing it doesn't ask the engine again.
  }, [version])

  useEffect(() => {
    nameRef.current?.select()
  }, [])

  const form: NewProjectForm = useMemo(
    () => ({
      name,
      location,
      template,
      setup,
      ide: ide || null,
      git,
      modding,
      steamAppId: steamAppId.trim() ? Number(steamAppId) : null
    }),
    [name, location, template, setup, ide, git, modding, steamAppId]
  )

  // Check the name and folder as they're typed.
  useEffect(() => {
    let current = true
    const timer = setTimeout(() => {
      api.checkNewProject(form).then(
        (problem) => current && setProblem(problem),
        () => {}
      )
    }, 120)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [form])

  const steamProblem = steamAppId.trim() && !/^[1-9]\d{0,9}$/.test(steamAppId.trim()) ? 'a Steam App ID is a whole number' : null
  const shownProblem = steamProblem ?? problem
  const creating = steps !== null
  const canCreate = usable && offer !== null && !shownProblem && !creating

  const create = async () => {
    if (!canCreate) return
    setSteps([])
    setCreateError(null)
    try {
      const path = await api.createProject(version, form, (step) => setSteps((current) => [...(current ?? []), step]))
      const store = useStore.getState()
      close()
      await store.refresh()
      store.setView('projects')
      store.setHighlight(path)
      store.toast('ok', `Created ${name.trim()}`, {
        label: 'Open',
        run: () => {
          const project = useStore.getState().projects.find((p) => p.path === path)
          if (project) void openProject(project)
        }
      })
    } catch (error) {
      setSteps(null)
      setCreateError(failure(error).message)
    }
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !creating) close()
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) void create()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const chooseTemplate = (id: string) => {
    setTemplate(id)
    setSetup(offer?.templates.find((t) => t.id === id)?.setup ?? [])
  }

  const browse = async () => {
    const folder = await api.pickFolder('Where should the project go?', location || undefined)
    if (folder) setLocation(folder)
  }

  return (
    <div className="overlay" onMouseDown={() => !creating && close()}>
      <div className="dialog new-project" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="New project">
        <div className="dialog-head">
          <div className="dialog-title">New project</div>
          {usable && (
            <select className="input" style={{ width: 'auto', height: 28 }} value={version} onChange={(e) => setVersion(e.target.value)} disabled={creating} title="Engine version">
              {installs.map((install) => (
                <option key={install.version} value={install.version}>
                  Voxear {install.version}
                </option>
              ))}
            </select>
          )}
          <button className="icon-btn" title="Close" onClick={close} disabled={creating}>
            <Icon name="close" size={15} />
          </button>
        </div>

        {!usable ? (
          <NeedsEngine />
        ) : (
          <div className="new-project-body">
            <div className="templates">
              {offerError && (
                <div className="banner banner-error" style={{ marginBottom: 12 }}>
                  <Icon name="alert" size={15} />
                  <span>
                    <Rich text={offerError} />
                  </span>
                </div>
              )}
              <div className="template-grid">
                {(offer?.templates ?? []).map((t) => (
                  <button key={t.id} className={`template ${template === t.id ? 'on' : ''}`} onClick={() => chooseTemplate(t.id)} onDoubleClick={() => void create()} disabled={creating}>
                    <div className="template-art">
                      <TemplateArt id={t.id} />
                    </div>
                    <div className="template-text">
                      <span className="template-name">{t.name}</span>
                      <span className="template-desc">{t.description}</span>
                    </div>
                  </button>
                ))}
              </div>
            </div>

            <div className="form-panel">
              {creating ? (
                <div className="steps" aria-live="polite">
                  <span className="label">Creating {name.trim()}</span>
                  {steps.map((step, index) => (
                    <div key={index} className={`step ${index === steps.length - 1 ? 'current' : ''}`}>
                      <Icon name={index === steps.length - 1 ? 'spinner' : 'check'} size={14} strokeWidth={2} className={index === steps.length - 1 ? 'spin' : undefined} />
                      {step}
                    </div>
                  ))}
                  {steps.length === 0 && (
                    <div className="step current">
                      <Icon name="spinner" size={14} strokeWidth={2} className="spin" />
                      Starting vox new
                    </div>
                  )}
                </div>
              ) : (
                <>
                  <label className="field">
                    <span className="label">Name</span>
                    <input ref={nameRef} className="input" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void create()} autoFocus spellCheck={false} />
                  </label>
                  <div className="field">
                    <span className="label">Location</span>
                    <div className="input-row">
                      <input className="input" value={location} onChange={(e) => setLocation(e.target.value)} spellCheck={false} />
                      <button className="btn" onClick={() => void browse()}>
                        Browse
                      </button>
                    </div>
                    <span className="hint" title={joinPath(location, name.trim())}>
                      Creates {pretty(joinPath(location, name.trim() || '…'), info?.userHome)}
                    </span>
                  </div>

                  {offer && offer.setup.length > 0 && (
                    <div className="form-group">
                      <span className="label" style={{ marginBottom: 4 }}>
                        Default setup
                      </span>
                      {offer.setup.map((item) => {
                        const on = setup.includes(item.id)
                        return (
                          <button key={item.id} className="switch-row" onClick={() => setSetup(on ? setup.filter((s) => s !== item.id) : [...setup, item.id])}>
                            <span className="text">
                              {item.name}
                              {item.description && <small>{item.description}</small>}
                            </span>
                            <Switch on={on} />
                          </button>
                        )
                      })}
                    </div>
                  )}

                  <div className="form-group">
                    <button className={`disclosure ${extrasOpen ? 'open' : ''}`} onClick={() => setExtrasOpen(!extrasOpen)}>
                      <Icon name="chevron" size={13} strokeWidth={2} />
                      Extras
                      <span className="hint">{extrasSummary(ide, offer, git, modding, steamAppId)}</span>
                    </button>
                    {extrasOpen && (
                      <div className="reveal" style={{ display: 'grid', gap: 2, marginTop: 6 }}>
                        {offer && offer.ides.length > 0 && (
                          <label className="field" style={{ padding: '4px 10px 8px' }}>
                            <span className="label">C# IDE</span>
                            <select className="input" value={ide} onChange={(e) => setIde(e.target.value)}>
                              <option value="">None</option>
                              {offer.ides.map((choice) => (
                                <option key={choice.id} value={choice.id}>
                                  {choice.name}
                                </option>
                              ))}
                            </select>
                          </label>
                        )}
                        <button className="switch-row" onClick={() => setGit(!git)}>
                          <span className="text">
                            Git repository
                            <small>With a .gitignore for Voxear</small>
                          </span>
                          <Switch on={git} />
                        </button>
                        <button className="switch-row" onClick={() => setModding(!modding)}>
                          <span className="text">
                            Modding
                            <small>Let players load mods</small>
                          </span>
                          <Switch on={modding} />
                        </button>
                        <label className="field" style={{ padding: '8px 10px 4px' }}>
                          <span className="label">Steam App ID</span>
                          <input className="input" value={steamAppId} placeholder="Optional, such as 480" inputMode="numeric" onChange={(e) => setSteamAppId(e.target.value)} />
                        </label>
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>
          </div>
        )}

        {usable && (
          <div className="dialog-foot">
            <span className={createError || shownProblem ? 'error-text' : 'hint'} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={createError ?? shownProblem ?? undefined}>
              <Rich text={createError ?? (shownProblem ? capitalise(shownProblem) : `Made by Voxear ${version}'s own \`vox new\``)} />
            </span>
            <div className="spacer" />
            <button className="btn btn-ghost" onClick={close} disabled={creating}>
              Cancel
            </button>
            <button className="btn btn-primary" onClick={() => void create()} disabled={!canCreate} title="Ctrl Enter">
              {creating ? 'Creating…' : 'Create project'}
            </button>
          </div>
        )}
      </div>
    </div>
  )
}

function extrasSummary(ide: string, offer: Offer | null, git: boolean, modding: boolean, steamAppId: string): string {
  const parts = [
    ide ? offer?.ides.find((choice) => choice.id === ide)?.name : null,
    git ? 'Git' : null,
    modding ? 'Modding' : null,
    steamAppId.trim() ? 'Steam' : null
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : 'None'
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function NeedsEngine() {
  const setInstallSheet = useStore((s) => s.setInstallSheet)
  const close = useStore((s) => s.closeNewProject)
  return (
    <div className="empty" style={{ margin: '12vh auto' }}>
      <h2>Install an engine version first</h2>
      <p>New projects are made by the engine version they use, so the launcher needs one installed.</p>
      <div className="empty-actions">
        <button
          className="btn btn-primary"
          onClick={() => {
            close()
            setInstallSheet(true)
          }}
        >
          <Icon name="download" size={15} />
          Install a version
        </button>
      </div>
    </div>
  )
}
