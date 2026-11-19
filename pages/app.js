/* ==========================================================================
   EunomiaSync — main window prototype: light vanilla-JS interactivity.
   Search, project filter, "Show hidden", row selection, section collapse,
   instruction editor modal, dirty-state tracking. No deps, no build step.
   ========================================================================== */

;(() => {
  'use strict'

  const tbody = document.querySelector('.matrix tbody')
  const searchInput = document.getElementById('search')
  const filterSelect = document.getElementById('proj-filter')
  const showHiddenToggle = document.getElementById('show-hidden')
  const selectAll = document.getElementById('select-all')
  const selectedChip = document.getElementById('selected-chip')
  const emptyState = document.getElementById('empty-state')
  const dialog = document.getElementById('placeholder-dialog')
  const dialogTitle = document.getElementById('dlg-title')
  const dialogText = document.getElementById('dlg-text')
  const editDialog = document.getElementById('edit-dialog')
  const editTitle = document.getElementById('edit-title')
  const editText = document.getElementById('edit-text')
  const editVariants = document.getElementById('edit-variants')
  const editVariantsSummary = document.getElementById('edit-variants-summary')
  const editVariantsList = document.getElementById('edit-variants-list')
  const editLabel = document.getElementById('edit-text-label')
  const runSyncBtn = document.getElementById('run-sync')
  const syncBadge = document.querySelector('.syncbar .badge')
  const syncCounterText = document.getElementById('sync-counter-text')
  const hiddenCountChip = document.getElementById('hidden-count')

  /* Project columns: skip the pinned checkbox + instruction columns */
  const projectCols = [...document.querySelectorAll('.matrix colgroup col')].slice(2)
  const projectNames = [...filterSelect.options]
    .slice(1)
    .map((o) => (o.value || o.textContent).trim())

  const rows = () => [...tbody.rows]
  const isHeading = (tr) => tr.classList.contains('row-heading')
  const headingLevel = (tr) => Number(tr.querySelector('[data-level]').dataset.level)
  const instructionName = (tr) =>
    (tr.querySelector('.name')?.textContent ?? '').trim().toLowerCase()

  /* Collapsed heading <tr> elements */
  const collapsed = new Set()

  /* ---------- Row visibility: search + "Show hidden" + collapse ---------- */

  function applyFilters() {
    const term = searchInput.value.trim().toLowerCase()
    const searching = term !== ''
    const showHidden = showHiddenToggle.checked

    /* Instruction rows match by name; hidden ones only with the toggle on */
    const matches = new Map()
    for (const tr of rows()) {
      if (isHeading(tr)) continue
      const byName = !searching || instructionName(tr).includes(term)
      const byHidden = showHidden || !tr.classList.contains('row-hidden')
      matches.set(tr, byName && byHidden)
    }

    /* Heading -> descendant instructions (needed for the search mode) */
    const children = new Map()
    let stack = []
    for (const tr of rows()) {
      if (isHeading(tr)) {
        while (stack.length && headingLevel(stack.at(-1)) >= headingLevel(tr)) {
          stack.pop()
        }
        stack.push(tr)
        children.set(tr, [])
      } else {
        for (const h of stack) children.get(h).push(tr)
      }
    }

    /* Apply. While searching, collapse state is ignored (search overrides). */
    stack = []
    let visibleInstructions = 0
    for (const tr of rows()) {
      if (isHeading(tr)) {
        while (stack.length && headingLevel(stack.at(-1)) >= headingLevel(tr)) {
          stack.pop()
        }
        stack.push(tr)
        const insideCollapsed = stack.slice(0, -1).some((h) => collapsed.has(h))
        const visible = searching ? children.get(tr).some((r) => matches.get(r)) : !insideCollapsed
        setVisible(tr, visible)
      } else {
        const insideCollapsed = stack.some((h) => collapsed.has(h))
        const visible = matches.get(tr) && (searching || !insideCollapsed)
        setVisible(tr, visible)
        if (visible) visibleInstructions += 1
      }
    }

    emptyState.hidden = visibleInstructions !== 0
    syncSelectionUI()
  }

  function setVisible(tr, visible) {
    tr.classList.toggle('row-off', !visible)
  }

  /* ---------- Column visibility: project filter (D-14) -------------------- */

  function applyColumnFilter() {
    const value = filterSelect.value // "all" or a project name
    const index = value === 'all' ? -1 : projectNames.indexOf(value)
    projectCols.forEach((col, i) => col.classList.toggle('col-off', index !== -1 && i !== index))
  }

  /* ---------- Row selection (leading checkboxes) --------------------------- */

  function visibleRowChecks() {
    return rows()
      .filter((tr) => !tr.classList.contains('row-off'))
      .map((tr) => tr.querySelector('.row-sel'))
      .filter(Boolean)
  }

  function syncSelectionUI() {
    const checks = visibleRowChecks()
    const checked = checks.filter((cb) => cb.checked)
    selectAll.checked = checks.length > 0 && checked.length === checks.length
    selectAll.indeterminate = checked.length > 0 && checked.length < checks.length

    const n = checked.length
    selectedChip.hidden = n === 0
    if (n > 0) {
      selectedChip.textContent = n === 1 ? '1 row selected' : `${n} rows selected`
    }
  }

  /* ---------- Changed cells: assignment differs from the last sync -------- */
  /* data-initial="true|false" (in the markup) records the state at the last
     sync; a checkbox that differs from it tints its cell until "Run sync"
     applies pending changes. */

  function updateCellState(input) {
    if (!('initial' in input.dataset)) {
      input.dataset.initial = String(input.checked)
    }
    const changed = (input.dataset.initial === 'true') !== input.checked
    const td = input.closest('td')
    td.classList.toggle('cell-changed', changed)
    td.title = changed ? 'Assignment changed — will be written on the next sync' : ''
  }

  /* ---------- Dirty state: assignments -> sync counter + column status ---- */
  /* MAIN-026/028: a project is "unsynced changes" while at least one
     assignment differs from data-initial; when every change reverts, the
     previous status (warning/unavailable) or the last sync time returns.
     "Run sync" applies pending changes and resets the demo state. */

  const projectHeads = [...document.querySelectorAll('thead th.c-proj')]

  const projectColsState = projectHeads.map((th) => {
    const sub = th.querySelector('.proj-sub')
    const initiallyDirty = sub.classList.contains('st-dirty')
    const m = (th.getAttribute('title') || '').match(/(\d{1,2} \w{3}) \d{4}, (\d{2}:\d{2})/)
    return {
      sub,
      dirty: initiallyDirty,
      /* How a clean column looks: warning/unavailable keep their markup,
         previously dirty columns fall back to the plain sync time. */
      cleanClass: initiallyDirty ? 'proj-sub' : sub.className,
      cleanHtml: initiallyDirty ? '' : sub.innerHTML,
      time: m ? `${m[1]}, ${m[2]}` : '',
    }
  })

  function renderSyncStatus() {
    for (const c of projectColsState) {
      if (c.dirty) {
        c.sub.className = 'proj-sub st-dirty'
        c.sub.innerHTML = '<span class="dot" aria-hidden="true"></span>unsynced changes'
      } else if (c.cleanHtml) {
        c.sub.className = c.cleanClass
        c.sub.innerHTML = c.cleanHtml
      } else {
        c.sub.className = 'proj-sub'
        c.sub.textContent = c.time
      }
    }
    const n = projectColsState.filter((c) => c.dirty).length
    syncBadge.textContent = n
    syncCounterText.textContent =
      n === 1 ? '1 project with unsynced changes' : `${n} projects with unsynced changes`
  }

  /* Dirty = the column has at least one assignment differing from data-initial */
  function refreshDirtyState() {
    const changed = new Array(projectColsState.length).fill(false)
    for (const input of document.querySelectorAll('.matrix tbody .assign')) {
      const idx = input.closest('td').cellIndex - 2
      if (idx < 0 || idx >= changed.length) continue
      changed[idx] = changed[idx] || (input.dataset.initial === 'true') !== input.checked
    }
    projectColsState.forEach((c, i) => (c.dirty = changed[i]))
    renderSyncStatus()
  }

  runSyncBtn.addEventListener('click', () => {
    /* Sync applies pending assignments: changed cells become the new
       initial state and lose the tint */
    for (const input of document.querySelectorAll('.matrix tbody .assign')) {
      input.dataset.initial = String(input.checked)
      updateCellState(input)
    }
    refreshDirtyState()
  })

  /* ---------- Section collapse/expand (headings + carets) ----------------- */

  tbody.addEventListener('click', (e) => {
    const heading = e.target.closest('tr.row-heading')
    if (!heading) return
    const nowCollapsed = !collapsed.has(heading)
    if (nowCollapsed) collapsed.add(heading)
    else collapsed.delete(heading)
    heading.querySelector('.caret-btn').setAttribute('aria-expanded', String(!nowCollapsed))
    applyFilters()
  })

  /* ---------- Instruction editor modal (UI-003, LIB-002) ------------------- */

  /* All variants per instruction; the accordion lists exactly these,
     each with its own editable textarea. Count matches the badge. */
  const VARIANTS = {
    'Language of responses': [
      {
        name: 'Spanish',
        text: 'Responde siempre en el idioma del usuario; por defecto, en español.',
      },
      { name: 'Mandarin', text: '始终以用户的语言回答；默认为中文。' },
    ],
    'Formatting and linting': [
      {
        name: 'Main',
        text: 'The formatter is the source of truth; never argue with it.',
      },
      {
        name: 'Per-project configs',
        text: "Respect the project's own formatter config when it exists.",
      },
    ],
    'Naming conventions': [
      {
        name: 'Main',
        text: 'Descriptive names; no single-letter identifiers outside loop counters.',
      },
      {
        name: 'Strict',
        text: 'No abbreviations at all; names are full words, even in loops.',
      },
    ],
    'Commit messages': [
      {
        name: 'Main',
        text: 'Imperative mood, subject up to 72 characters; the body explains why.',
      },
      {
        name: 'Conventional Commits',
        text: 'type(scope): subject — feat, fix, docs, refactor, test, chore.',
      },
    ],
    'Bug-driven development': [
      {
        name: 'Main',
        text: 'A red test first, then the fix that makes it green.',
      },
      {
        name: 'Bug-first',
        text: 'Reproduce the bug in a failing test committed before the fix.',
      },
    ],
    'PR review checklist': [
      {
        name: 'Main',
        text: 'Check behavior, tests, docs and DECISIONS entries before approving.',
      },
      {
        name: 'Lightweight',
        text: 'Review only the diff: behavior and tests.',
      },
    ],
  }

  const escapeHtml = (s) =>
    s.replace(
      /[&<>"']/g,
      (ch) =>
        ({
          '&': '&amp;',
          '<': '&lt;',
          '>': '&gt;',
          '"': '&quot;',
          "'": '&#39;',
        })[ch],
    )

  tbody.addEventListener('click', (e) => {
    const btn = e.target.closest('.edit-btn')
    if (!btn) return
    const row = btn.closest('tr')
    const name = (row.querySelector('.name')?.textContent ?? '').trim()
    const isEmpty = !!row.querySelector('.name-empty')

    editTitle.textContent = `Edit instruction — ${name}`
    editText.value = isEmpty ? '' : (row.querySelector('.c-name')?.getAttribute('title') ?? '')
    editText.placeholder = isEmpty
      ? 'No text yet — write the main variant here.'
      : 'Instruction text (Markdown)…'

    const variants = VARIANTS[name] ?? []
    if (variants.length >= 2) {
      editVariants.hidden = false
      editVariants.open = false // collapsed by default every time
      editVariantsSummary.textContent = `Variants (${variants.length})`
      editVariantsList.innerHTML = variants
        .map(
          (v, i) =>
            `<div class="variant-item">` +
            `<label class="variant-name" for="variant-text-${i}">${escapeHtml(v.name)}</label>` +
            `<textarea id="variant-text-${i}" class="variant-textarea" rows="2">${escapeHtml(v.text)}</textarea>` +
            `</div>`,
        )
        .join('')
      editLabel.textContent = 'Text'
    } else {
      editVariants.hidden = true
      editLabel.textContent = 'Text (main variant)'
    }
    editDialog.showModal()
  })

  /* ---------- Toolbar wiring ----------------------------------------------- */

  searchInput.addEventListener('input', applyFilters)
  showHiddenToggle.addEventListener('change', applyFilters)
  filterSelect.addEventListener('change', applyColumnFilter)

  /* ---------- Checkbox changes (selection + assignments) ------------------- */

  tbody.addEventListener('change', (e) => {
    const sel = e.target.closest('.row-sel')
    if (sel) {
      sel.closest('tr').classList.toggle('row-selected', sel.checked)
      syncSelectionUI()
      return
    }
    const assign = e.target.closest('.assign')
    if (assign && !assign.disabled) {
      updateCellState(assign)
      refreshDirtyState()
    }
  })

  /* ---------- Placeholder dialogs (native <dialog>) ------------------------ */

  for (const btn of document.querySelectorAll('.js-open-dialog')) {
    btn.addEventListener('click', () => {
      dialogTitle.textContent = btn.dataset.dialogTitle
      dialogText.textContent = btn.dataset.dialogText
      dialog.showModal()
    })
  }

  /* ---------- Init ----------------------------------------------------------*/

  /* Rows checked in the markup (default selection) get the selected
     background too, not only rows the user clicks */
  for (const tr of rows()) {
    const box = tr.querySelector('.row-sel')
    if (box) tr.classList.toggle('row-selected', box.checked)
  }

  /* Cells seeded in the markup with data-initial get the tint right away,
     and the dirty flags follow from the actual differences */
  for (const input of document.querySelectorAll('.matrix tbody .assign')) {
    updateCellState(input)
  }

  /* "Show hidden" badge: how many instructions are hidden */
  hiddenCountChip.textContent = String(
    document.querySelectorAll('.matrix tbody .row-hidden').length,
  )

  applyColumnFilter()
  applyFilters()
  refreshDirtyState()
})()
