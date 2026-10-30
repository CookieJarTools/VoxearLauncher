// Project actions shared by the list, its menus and the command palette.

import { api, failure, type Project } from '../api'
import { useStore } from '../store'

const store = () => useStore.getState()

function report(error: unknown) {
  store().toast('error', failure(error).message)
}

/** Opens a project, or explains what it needs first and offers to do it. */
export async function openProject(project: Project) {
  const { status } = project
  switch (status.kind) {
    case 'ready':
      try {
        await api.openProject(project.path)
        store().toast('ok', `Opening ${project.name} with Voxear ${project.engine}`)
        void store().refresh()
      } catch (error) {
        report(error)
      }
      return
    case 'engineMissing': {
      const ok = await store().confirm({
        title: `Install Voxear ${status.version}?`,
        message: `${project.name} is pinned to Voxear ${status.version}, which isn't installed. Install it now? You can open the project once it's done.`,
        confirm: `Install ${status.version}`
      })
      if (ok) store().queueInstall(status.version)
      return
    }
    case 'unpinned':
      store().toast('info', `${project.name} isn't pinned to an engine version yet. Add engine = "<version>" under [project] in its voxear.toml.`)
      return
    case 'missing':
      store().toast('error', `${project.name}'s folder isn't there any more.`, { label: 'Remove from list', run: () => void removeProject(project) })
      return
    case 'unreadable':
      store().toast('error', status.reason)
      return
  }
}

export async function addProject() {
  const folder = await api.pickFolder('Add a Voxear project', store().info?.defaultLocation)
  if (!folder) return
  try {
    const name = await api.addProject(folder)
    await store().refresh()
    store().setView('projects')
    store().setHighlight(store().projects.find((p) => p.name === name)?.path ?? null)
    store().toast('ok', `Added ${name}`)
  } catch (error) {
    report(error)
  }
}

export async function removeProject(project: Project) {
  try {
    await api.removeProject(project.path)
    await store().refresh()
    store().toast('ok', `Took ${project.name} off the list. Its files are untouched.`)
  } catch (error) {
    report(error)
  }
}

export async function toggleFavourite(project: Project) {
  try {
    await api.setFavourite(project.path, !project.favourite)
    await store().refresh()
  } catch (error) {
    report(error)
  }
}

export async function reveal(path: string) {
  try {
    await api.reveal(path)
  } catch (error) {
    report(error)
  }
}
