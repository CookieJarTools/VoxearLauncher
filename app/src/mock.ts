// An in-memory stand-in for the Rust core, used when the UI runs in a plain browser. It behaves like the real
// commands closely enough to preview every screen: installs take a few seconds, and `vox new` reports steps.

import type { Api, Install, NewProjectForm, Offer, Progress, Project, Release, Settings } from './api'

const home = '/home/you'
const engines = `${home}/.local/share/voxear/engines`
const hour = 3600_000
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

const offer: Offer = {
  format: 1,
  templates: [
    { id: 'empty', name: 'Empty', description: 'Nothing but a camera', setup: ['camera'] },
    { id: '3d', name: '3D', description: 'A lit 3D scene', setup: ['lighting', 'camera', 'input', 'sample-scene', 'quality'] },
    { id: '2d', name: '2D', description: 'A 2D scene with a pixel-perfect camera', setup: ['camera', 'input', 'sample-scene'] },
    { id: 'first-person', name: 'First-person', description: 'A first-person controller', setup: ['lighting', 'camera', 'input', 'sample-scene', 'quality'] },
    { id: 'third-person', name: 'Third-person', description: 'A character and a follow camera', setup: ['lighting', 'camera', 'input', 'sample-scene', 'quality'] },
    { id: 'top-down', name: 'Top-down', description: 'A top-down camera and click-to-move', setup: ['lighting', 'camera', 'input', 'sample-scene', 'quality'] }
  ],
  setup: [
    { id: 'lighting', name: 'Lighting and sky', description: 'A sun, sky and fog' },
    { id: 'camera', name: 'Camera', description: 'A camera that suits the template' },
    { id: 'input', name: 'Input actions', description: 'Move, look, jump and interact' },
    { id: 'sample-scene', name: 'Sample scene', description: 'Something to walk around in' },
    { id: 'quality', name: 'Quality presets', description: 'Low, medium, high and ultra' }
  ],
  ides: [
    { id: 'rider', name: 'Rider', description: '' },
    { id: 'visual-studio', name: 'Visual Studio', description: '' },
    { id: 'vscode', name: 'VS Code', description: '' }
  ]
}

let installs: Install[] = [
  { version: '0.3.0-beta.1', channel: 'beta', path: `${engines}/0.3.0-beta.1`, located: false, missing: false, installed: ago(30 * hour) },
  { version: '0.2.0', channel: 'stable', path: `${engines}/0.2.0`, located: false, missing: false, installed: ago(9 * 24 * hour) },
  { version: '0.4.0-dev', channel: 'nightly', path: `${home}/Dev/Voxear/dist`, located: true, missing: false, installed: null }
]

const releases: Omit<Release, 'installed'>[] = [
  { version: '0.3.0-nightly.20261030', channel: 'nightly', published: ago(5 * hour), notes: null, size: 418_000_000, needsNewerLauncher: false },
  { version: '0.3.0-nightly.20261029', channel: 'nightly', published: ago(29 * hour), notes: null, size: 417_000_000, needsNewerLauncher: false },
  { version: '0.3.0-beta.1', channel: 'beta', published: ago(2 * 24 * hour), notes: 'https://example.invalid/0.3.0-beta.1', size: 405_000_000, needsNewerLauncher: false },
  { version: '0.2.1', channel: 'stable', published: ago(4 * 24 * hour), notes: 'https://example.invalid/0.2.1', size: 392_000_000, needsNewerLauncher: false },
  { version: '0.2.0', channel: 'stable', published: ago(20 * 24 * hour), notes: 'https://example.invalid/0.2.0', size: 391_000_000, needsNewerLauncher: false },
  { version: '0.1.0', channel: 'stable', published: ago(60 * 24 * hour), notes: null, size: 350_000_000, needsNewerLauncher: false }
]

let projects: Project[] = [
  { path: `${home}/Games/Orbit`, name: 'Orbit', engine: '0.2.0', status: { kind: 'ready' }, favourite: true, added: ago(40 * 24 * hour), opened: ago(2 * hour) },
  { path: `${home}/Games/Lantern`, name: 'Lantern', engine: '0.3.0-beta.1', status: { kind: 'ready' }, favourite: false, added: ago(12 * 24 * hour), opened: ago(26 * hour) },
  { path: `${home}/Games/Skate Park`, name: 'Skate Park', engine: '0.1.0', status: { kind: 'engineMissing', version: '0.1.0' }, favourite: false, added: ago(70 * 24 * hour), opened: ago(18 * 24 * hour) },
  { path: `${home}/Dev/Voxear/samples/Hello`, name: 'Hello', engine: null, status: { kind: 'unpinned' }, favourite: false, added: ago(3 * 24 * hour), opened: null },
  { path: '/run/media/you/USB/Tidepool', name: 'Tidepool', engine: '0.2.0', status: { kind: 'missing' }, favourite: false, added: ago(90 * 24 * hour), opened: ago(60 * 24 * hour) }
]

let settings: Settings = { releases: 'https://releases.example.invalid/voxear/releases.toml' }
const cancelled = new Set<string>()

function refreshStatuses() {
  projects = projects.map((project) => {
    if (project.status.kind === 'missing' || !project.engine) return project
    const ready = installs.some((install) => install.version === project.engine)
    return { ...project, status: ready ? { kind: 'ready' } : { kind: 'engineMissing', version: project.engine } }
  })
}

async function simulateInstall(key: string, size: number, onProgress: (progress: Progress) => void) {
  const steps = 40
  for (let i = 0; i <= steps; i++) {
    if (cancelled.delete(key)) throw { message: 'cancelled', cancelled: true }
    onProgress({ phase: 'downloading', done: Math.round((size * i) / steps), total: size })
    await wait(70)
  }
  onProgress({ phase: 'verifying' })
  await wait(500)
  for (let i = 0; i <= 10; i++) {
    onProgress({ phase: 'unpacking', done: i, total: 10 })
    await wait(60)
  }
  onProgress({ phase: 'finishing' })
  await wait(250)
}

function sortInstalls() {
  installs.sort((a, b) => b.version.localeCompare(a.version, undefined, { numeric: true }))
}

export const mockApi: Api = {
  appInfo: async () => ({
    version: '0.1.0',
    platform: 'linux-x86_64',
    home: `${home}/.local/share/voxear`,
    userHome: home,
    settingsFile: `${home}/.local/share/voxear/settings.toml`,
    defaultLocation: settings.project_location ?? `${home}/VoxearProjects`
  }),
  listInstalls: async () => installs.map((install) => ({ ...install })),
  listReleases: async () => {
    await wait(350)
    return {
      source: settings.releases ?? null,
      releases: releases.map((release) => ({ ...release, installed: installs.some((i) => i.version === release.version) })),
      stale: null
    }
  },
  install: async (version, onProgress) => {
    const release = releases.find((r) => r.version === version) ?? releases.find((r) => r.channel === version)
    if (!release) throw { message: `Voxear ${version} isn't in the release list`, cancelled: false }
    await simulateInstall(version, release.size, onProgress)
    const install: Install = { version: release.version, channel: release.channel, path: `${engines}/${release.version}`, located: false, missing: false, installed: new Date().toISOString() }
    installs.push(install)
    sortInstalls()
    refreshStatuses()
    return install
  },
  cancelInstall: async (version) => void cancelled.add(version),
  installFile: async (path, onProgress) => {
    await simulateInstall(path, 300_000_000, onProgress)
    const install: Install = { version: '0.5.0-beta.1', channel: 'beta', path: `${engines}/0.5.0-beta.1`, located: false, missing: false, installed: new Date().toISOString() }
    installs.push(install)
    sortInstalls()
    return install
  },
  locate: async (path) => {
    const install: Install = { version: '0.4.1-dev', channel: 'nightly', path, located: true, missing: false, installed: null }
    installs.push(install)
    sortInstalls()
    return install
  },
  projectsUsing: async (version) => projects.filter((p) => p.engine === version).map((p) => p.name),
  uninstall: async (version) => {
    const install = installs.find((i) => i.version === version)
    installs = installs.filter((i) => i.version !== version)
    refreshStatuses()
    return { forgotten: Boolean(install?.located) }
  },
  installSize: async (path) => (path.includes('/Dev/') ? 2_140_000_000 : 1_180_000_000),
  listProjects: async () => {
    refreshStatuses()
    return [...projects].sort((a, b) => Number(b.favourite) - Number(a.favourite) || (b.opened ?? b.added).localeCompare(a.opened ?? a.added))
  },
  addProject: async (path) => {
    const name = path.split('/').filter(Boolean).pop() ?? 'Project'
    projects.push({ path, name, engine: '0.2.0', status: { kind: 'ready' }, favourite: false, added: new Date().toISOString(), opened: null })
    return name
  },
  removeProject: async (path) => void (projects = projects.filter((p) => p.path !== path)),
  setFavourite: async (path, favourite) => void (projects = projects.map((p) => (p.path === path ? { ...p, favourite } : p))),
  openProject: async (path) => {
    const project = projects.find((p) => p.path === path)
    throw { message: `Voxear ${project?.engine} has no editor yet (it arrives with engine milestone M3)`, cancelled: false }
  },
  describeNew: async () => {
    await wait(150)
    return offer
  },
  checkNewProject: async (form: NewProjectForm) => {
    if (!form.name.trim()) return 'the project needs a name'
    if (/[/\\:*?"<>|]/.test(form.name)) return `project names can't contain \`${form.name.match(/[/\\:*?"<>|]/)?.[0]}\``
    if (projects.some((p) => p.path === `${form.location}/${form.name.trim()}`)) return `${form.location}/${form.name.trim()} already exists`
    return null
  },
  createProject: async (version, form, onStep) => {
    const template = offer.templates.find((t) => t.id === form.template)
    for (const step of [`Copying the ${template?.name ?? form.template} template`, 'Writing input actions', 'Writing voxear.toml', ...(form.git ? ['Starting a git repository'] : [])]) {
      onStep(step)
      await wait(420)
    }
    const path = `${form.location}/${form.name.trim()}`
    projects.push({ path, name: form.name.trim(), engine: version, status: { kind: 'ready' }, favourite: false, added: new Date().toISOString(), opened: null })
    settings = { ...settings, project_location: form.location }
    return path
  },
  getSettings: async () => ({ ...settings }),
  saveSettings: async (next) => void (settings = { ...next }),
  pickFolder: async (_title, defaultPath) => window.prompt('Folder', defaultPath ?? home),
  pickFile: async () => window.prompt('Archive', `${home}/Downloads/voxear-0.5.0-beta.1-linux-x86_64.tar.zst`),
  reveal: async () => {},
  openUrl: async (url) => void window.open(url, '_blank'),
  minimize: () => {},
  toggleMaximize: () => {},
  close: () => {}
}
