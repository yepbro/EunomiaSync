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
  const runSyncBtn = document.getElementById('run-sync')
  const syncBadge = document.querySelector('.syncbar .badge')
  const syncCounterText = document.getElementById('sync-counter-text')
  const hiddenCountChip = document.getElementById('hidden-count')
  const confirmDialog = document.getElementById('confirm-dialog')
  const confirmTitle = document.getElementById('confirm-title')
  const confirmText = document.getElementById('confirm-text')

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
  const instructionDisplay = (row) => (row.querySelector('.name')?.textContent ?? '').trim()
  const instructionText = (row) => {
    const isEmpty = !!row.querySelector('.name-empty')
    return isEmpty ? '' : (row.querySelector('.c-name')?.getAttribute('title') ?? '')
  }
  /* Text of a cell's currently selected option: the main instruction text
     for "Default", the mapped variant text otherwise */
  function optionText(row, option) {
    if (option === 'Default') return instructionText(row)
    const mapped = VARIANTS[instructionDisplay(row)]
    const found = mapped?.find((v) => v.name === option)
    if (found) return found.text
    return instructionText(row) + ' Slightly reworded version of the main text.'
  }

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
    const td = input.closest('td')
    const select = td.querySelector('.variant-select')
    if (select && !('initialVariant' in select.dataset)) {
      select.dataset.initialVariant = select.value
    }
    const checkedChanged = (input.dataset.initial === 'true') !== input.checked
    const variantChanged = !!select && select.dataset.initialVariant !== select.value
    const custom = td.dataset.custom === 'true'
    const changed = custom || checkedChanged || variantChanged
    td.classList.toggle('cell-changed', changed)
    td.title = changed ? 'Assignment changed — will be written on the next sync' : ''
  }

  /* Variant option list for a row: "Default" (the main instruction text)
     plus the instruction's actual variants — mapped ones keep their own
     names (they may or may not include "Variant 1"); badge rows without
     a map have the single default "Variant 1" */
  function variantOptionsFor(row) {
    const name = (row.querySelector('.name')?.textContent ?? '').trim()
    const mapped = VARIANTS[name]
    if (!mapped) return row.querySelector('.chip') ? ['Default', 'Variant 1'] : []
    return ['Default', ...mapped.map((v) => v.name)]
  }

  /* Every cell of a variant row gets checkbox + select; cells that had a
     variant-btn get a checked checkbox and keep that variant preselected */
  function buildVariantSelects() {
    for (const row of rows()) {
      if (isHeading(row)) continue
      const options = variantOptionsFor(row)
      if (!options.length) continue
      const instr = (row.querySelector('.name')?.textContent ?? '').trim()
      for (const td of row.querySelectorAll('td.c-cell')) {
        let input = td.querySelector('.assign')
        const prevBtn = td.querySelector('.variant-btn')
        const fromBtn = (prevBtn?.getAttribute('title') || '').match(/^Variant: (.+)$/)
        if (!input) {
          if (!prevBtn) continue
          /* former variant-btn cells had no checkbox: the alternative was
             assigned, so the checkbox starts checked */
          input = document.createElement('input')
          input.type = 'checkbox'
          input.className = 'assign'
          input.checked = true
          input.dataset.initial = 'true'
          const proj = document
            .querySelectorAll('thead th')
            [td.cellIndex]?.querySelector('.proj-name')
            ?.textContent.trim()
          input.setAttribute(
            'aria-label',
            `${instr} — ${proj}: variant ${fromBtn ? fromBtn[1] : options[0]} (changed)`,
          )
        }
        const selected = fromBtn && options.includes(fromBtn[1]) ? fromBtn[1] : options[0]
        prevBtn?.remove()
        const select = document.createElement('select')
        select.className = 'variant-select'
        select.setAttribute('aria-label', `Assigned text — ${instr}`)
        if (input.disabled) select.disabled = true
        for (const optName of options) {
          const opt = document.createElement('option')
          opt.value = optName
          opt.textContent = optName
          if (optName === selected) opt.selected = true
          select.append(opt)
        }
        const wrap = document.createElement('span')
        wrap.className = 'cell-assign'
        td.append(wrap)
        wrap.append(input, select)
      }
    }
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
      const td = input.closest('td')
      const idx = td.cellIndex - 2
      if (idx < 0 || idx >= changed.length) continue
      const select = td.querySelector('.variant-select')
      const cellChanged =
        td.dataset.custom === 'true' ||
        (input.dataset.initial === 'true') !== input.checked ||
        (!!select && select.dataset.initialVariant !== select.value)
      changed[idx] = changed[idx] || cellChanged
    }
    projectColsState.forEach((c, i) => (c.dirty = changed[i]))
    renderSyncStatus()
  }

  runSyncBtn.addEventListener('click', () => {
    /* Sync applies pending assignments: changed cells become the new
       initial state and lose the tint */
    for (const input of document.querySelectorAll('.matrix tbody .assign')) {
      const select = input.closest('td').querySelector('.variant-select')
      if (select) select.dataset.initialVariant = select.value
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
        name: 'Variant 1',
        text: 'The formatter is the source of truth; never argue with it.',
      },
      {
        name: 'Per-project configs',
        text: "Respect the project's own formatter config when it exists.",
      },
    ],
    'Naming conventions': [
      {
        name: 'Variant 1',
        text: 'Descriptive names; no single-letter identifiers outside loop counters.',
      },
      {
        name: 'Strict',
        text: 'No abbreviations at all; names are full words, even in loops.',
      },
    ],
    'Commit messages': [
      {
        name: 'Variant 1',
        text: 'Imperative mood, subject up to 72 characters; the body explains why.',
      },
      {
        name: 'Conventional Commits',
        text: 'type(scope): subject — feat, fix, docs, refactor, test, chore.',
      },
    ],
    'Bug-driven development': [
      {
        name: 'Variant 1',
        text: 'A red test first, then the fix that makes it green.',
      },
      {
        name: 'Bug-first',
        text: 'Reproduce the bug in a failing test committed before the fix.',
      },
    ],
    'PR review checklist': [
      {
        name: 'Variant 1',
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

  function openEditModal({ title, text, placeholder, variants }) {
    const list = variants ?? []
    editTitle.textContent = title
    editText.value = text
    editText.placeholder = placeholder
    editVariants.hidden = list.length === 0
    editVariants.open = false // collapsed by default every time
    editVariantsSummary.textContent = `Variants (${list.length})`
    editVariantsList.innerHTML = list
      .map(
        (v, i) =>
          `<div class="variant-item">` +
          `<label class="variant-name" for="variant-text-${i}">${escapeHtml(v.name)}</label>` +
          `<textarea id="variant-text-${i}" class="variant-textarea" rows="2">${escapeHtml(v.text)}</textarea>` +
          `</div>`,
      )
      .join('')
    editDialog.showModal()
  }

  tbody.addEventListener('click', (e) => {
    const btn = e.target.closest('.edit-btn')
    if (!btn) return
    const row = btn.closest('tr')
    const name = instructionName(row)
    const mainText = instructionText(row)
    const isEmpty = !mainText

    /* The accordion lists every variant the instruction has: mapped ones
       explicitly; single-variant rows get their "Variant 1" synthesized from
       the instruction text; badge-less rows have no variants at all.
       "Variant 1" is the default wording of the main text: nudge it a bit
       so the variant visibly differs from the field above */
    const variantCount = Number(row.querySelector('.chip')?.textContent ?? 0)
    const variants = (
      VARIANTS[name] ?? (variantCount >= 1 ? [{ name: 'Variant 1', text: mainText }] : [])
    ).map((v) =>
      v.name === 'Variant 1' && v.text === mainText
        ? { name: v.name, text: v.text + ' Slightly reworded version of the main text.' }
        : v,
    )
    openEditModal({
      title: `Edit instruction — ${name}`,
      text: mainText,
      placeholder: isEmpty
        ? 'No text yet — write the main variant here.'
        : 'Instruction text (Markdown)…',
      variants,
    })
  })

  /* ---------- Per-project custom instructions (cell icons + flows) -------- */

  let pendingCustomCell = null
  let pendingDeleteCell = null
  const customTexts = new WeakMap()

  function refreshCellIcons(td) {
    const row = td.closest('tr')
    const instr = instructionDisplay(row)
    const proj =
      document
        .querySelectorAll('thead th')
        [td.cellIndex]?.querySelector('.proj-name')
        ?.textContent.trim() ?? ''
    const input = td.querySelector('.assign')
    const select = td.querySelector('.variant-select')
    const isCustom = td.dataset.custom === 'true'
    const columnUnavailable = !!input?.disabled && !isCustom

    let box = td.querySelector('.cell-icons')
    if (!select || columnUnavailable) {
      box?.remove()
      return
    }
    if (!box) {
      box = document.createElement('span')
      box.className = 'cell-icons'
      td.append(box)
    }
    const mk = (cls, title) => {
      const b = document.createElement('button')
      b.type = 'button'
      b.className = `cell-icon ${cls}`
      b.title = title
      b.setAttribute('aria-label', title)
      return b
    }
    box.replaceChildren()
    if (isCustom) {
      const edit = mk('cell-icon--edit', `Edit the custom instruction — ${instr} — ${proj}`)
      edit.addEventListener('click', () => {
        pendingCustomCell = td
        openEditModal({
          title: `Custom instruction — ${instr} — ${proj}`,
          text: customTexts.get(td) ?? '',
          placeholder: 'Write the custom instruction text…',
          variants: [],
        })
      })
      const del = mk('cell-icon--delete', `Delete the custom instruction — ${instr} — ${proj}`)
      del.addEventListener('click', () => {
        pendingDeleteCell = td
        confirmTitle.textContent = 'Delete custom instruction'
        confirmText.textContent = `Delete the custom instruction for ${instr} — ${proj}? The cell returns to the selected option of the list.`
        confirmDialog.showModal()
      })
      box.append(edit, del)
    } else {
      const add = mk('cell-icon--add', `Create a custom instruction — ${instr} — ${proj}`)
      add.addEventListener('click', () => {
        pendingCustomCell = td
        openEditModal({
          title: `Custom instruction — ${instr} — ${proj}`,
          text: optionText(row, select.value),
          placeholder: 'Write the custom instruction text…',
          variants: [],
        })
      })
      box.append(add)
    }
  }

  /* Saving from the custom flow applies the custom text to the cell:
     the select is disabled (there is nothing left to pick) while the
     checkbox stays enabled — the custom instruction can be toggled */
  editDialog.addEventListener('close', () => {
    const td = pendingCustomCell
    pendingCustomCell = null
    if (!td || editDialog.returnValue !== 'save') return
    const input = td.querySelector('.assign')
    const select = td.querySelector('.variant-select')
    customTexts.set(td, editText.value)
    td.dataset.custom = 'true'
    input.checked = true
    input.dataset.initial = 'true'
    if (select) {
      select.dataset.initialVariant = select.value
      select.disabled = true
    }
    refreshCellIcons(td)
    updateCellState(input)
    refreshDirtyState()
  })

  confirmDialog.addEventListener('close', () => {
    const td = pendingDeleteCell
    pendingDeleteCell = null
    if (!td || confirmDialog.returnValue !== 'delete') return
    delete td.dataset.custom
    customTexts.delete(td)
    const input = td.querySelector('.assign')
    const select = td.querySelector('.variant-select')
    input.disabled = false
    if (select) select.disabled = false
    refreshCellIcons(td)
    updateCellState(input)
    refreshDirtyState()
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
      return
    }
    const select = e.target.closest('.variant-select')
    if (select) {
      const input = select.closest('td').querySelector('.assign')
      if (input) updateCellState(input)
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

  /* Variant selects are synthesized first so every cell has one before
     the changed-state pass runs */
  buildVariantSelects()

  /* Per-project edit icon in the header: opens a placeholder modal only */
  for (const th of projectHeads) {
    const proj = th.querySelector('.proj-name')?.textContent.trim() ?? ''
    const line = th.querySelector('.proj-line')
    if (!line) continue
    const btn = document.createElement('button')
    btn.type = 'button'
    btn.className = 'cell-icon cell-icon--edit proj-edit'
    btn.title = `Edit project — ${proj}`
    btn.setAttribute('aria-label', `Edit project — ${proj}`)
    btn.addEventListener('click', () => {
      dialogTitle.textContent = `Edit project — ${proj}`
      dialogText.textContent =
        'Prototype placeholder: project editing (name, label color, description, sync settings — PRJ-008…) is not implemented in this prototype.'
      dialog.showModal()
    })
    line.append(btn)
  }

  /* Per-project custom instruction icons on every variant cell */
  for (const td of document.querySelectorAll('.matrix tbody td.c-cell')) {
    if (td.querySelector('.variant-select')) refreshCellIcons(td)
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
