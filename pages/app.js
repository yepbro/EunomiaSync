/* ==========================================================================
   EunomiaSync — main window prototype: light vanilla-JS interactivity.
   Search, project filter, "Show hidden", row selection, section collapse,
   instruction editor modal (UI-003: two-pane editor — variant list,
   Edit/Preview tabs, validation, Markdown preview, unsaved-changes guard),
   instruction history modal (LIB-026…027: revision list, snapshot
   disclosures, restore variant; D-16),
   project settings modal (PRJ-008…012, PRJ-013…018 archive confirm),
   repository scan modal (PRJ-002…007: scripted walk, live progress,
   three result groups; NFR-015 — closing the window cancels),
   sync summary modal (MAIN-024…031, OUT-009: simulated sync run with
   k-of-N progress and per-project outcomes per §6.2/D-09/SYS-009,
   pending-changes list from the counter chip; MAIN-030 guard),
   app settings modal (DEF-001…005: §6.5 SETTINGS demo object, controls
   apply immediately — no Save button, inline range errors, editable
   scan exclusions, toolbar button + Ctrl/Cmd+, per §11.3),
   archived projects modal (D-10, PRJ-013…018: list with a Restore
   action confirmed in a dedicated dialog; DB-014/015),
   excluded paths modal (D-10: list with a Remove action confirmed in
   a dedicated dialog; §6.2 project_exclusions, §6.4 physical delete),
   activity modal (§7.7 LOG-001…006: user-visible entries only, live
   date/project/type/result filters, bulk ops logged with a change
   count),
   help modal (HLP-001…005: two-pane FAQ — article list, client-side
   search with <mark> highlights in the list and the rendered body,
   articles rendered with the D-15 mini renderer; toolbar "?" button +
   F1 per §11.3),
   dirty-state tracking. No deps, no build step.
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
    td.classList.toggle('cell-unchecked', !input.checked)
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
    /* MAIN-030: the simulated run applies pending assignments project by
       project (the same reset the old instant-apply handler did, scoped
       per column) and ends with the summary modal — see the sync
       summary section at the bottom of the file */
    startSyncRun()
  })

  /* ---------- Row drag-and-drop between sections (MAIN-015…023) ---------- */
  /* Instructions move between sections; headings move with their whole
     content (LIB-032/034). A branch cannot be dropped inside itself:
     dropped on any other heading a section becomes its last child, and
     dropped on the "Instruction" column header it becomes the first
     top-level section. */

  function initRowDrag() {
    const table = document.querySelector('.matrix')
    const headerTh = document.getElementById('instruction-header')
    let dragged = null
    let draggedSectionRows = []
    let dropMark = null

    const clearDropMark = () => {
      dropMark?.classList.remove('drop-target')
      dropMark = null
    }

    const finishDrag = () => {
      dragged?.classList.remove('dragging')
      dragged = null
      draggedSectionRows = []
      clearDropMark()
      applyFilters()
    }

    /* First row after the heading's section (insertion point) */
    const sectionEndAfter = (headingRow) => {
      const level = headingLevel(headingRow)
      let tr = headingRow.nextElementSibling
      while (tr && !(tr.classList.contains('row-heading') && headingLevel(tr) <= level)) {
        tr = tr.nextElementSibling
      }
      return tr
    }

    /* All rows belonging to the heading's section (heading excluded) */
    const sectionRowsOf = (headingRow) => {
      const rowsList = []
      const level = headingLevel(headingRow)
      let tr = headingRow.nextElementSibling
      while (tr && !(tr.classList.contains('row-heading') && headingLevel(tr) <= level)) {
        rowsList.push(tr)
        tr = tr.nextElementSibling
      }
      return rowsList
    }

    /* Shift levels and indents of the moved rows by delta. --depth lives on
       the th.c-name (it drives the indent), not on the tr */
    const shiftLevels = (moving, delta) => {
      for (const row of moving) {
        const styleEl = row.querySelector('.c-name')
        const depth = Number(styleEl.style.getPropertyValue('--depth') || 0) + delta
        styleEl.style.setProperty('--depth', String(depth))
        if (isHeading(row)) {
          row.querySelector('[data-level]').dataset.level = String(headingLevel(row) + delta)
        }
      }
    }

    for (const tr of rows()) tr.draggable = true

    tbody.addEventListener('dragstart', (e) => {
      const tr = e.target.closest('tr')
      if (!tr || tr.classList.contains('row-hidden')) return
      dragged = tr
      draggedSectionRows = isHeading(tr) ? sectionRowsOf(tr) : []
      tr.classList.add('dragging')
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData(
        'text/plain',
        isHeading(tr)
          ? 'Section: ' + tr.querySelector('.hname').textContent.trim()
          : instructionDisplay(tr),
      )
    })

    tbody.addEventListener('dragend', () => {
      dragged?.classList.remove('dragging')
      dragged = null
      draggedSectionRows = []
      clearDropMark()
    })

    table.addEventListener('dragover', (e) => {
      if (!dragged) return
      if (e.target.closest('#instruction-header')) {
        if (!isHeading(dragged)) return
        e.preventDefault()
        e.dataTransfer.dropEffect = 'move'
        if (dropMark !== headerTh) {
          clearDropMark()
          dropMark = headerTh
          headerTh.classList.add('drop-target')
        }
        return
      }
      const target = e.target.closest('tr')
      const insideSelf = draggedSectionRows.includes(target) || target === dragged
      if (!target || insideSelf || target.classList.contains('row-hidden')) {
        clearDropMark()
        return
      }
      e.preventDefault()
      e.dataTransfer.dropEffect = 'move'
      if (dropMark !== target) {
        clearDropMark()
        dropMark = target
        dropMark.classList.add('drop-target')
      }
    })

    table.addEventListener('drop', (e) => {
      /* Dropped on the "Instruction" column header: the section becomes the
         very first top-level one */
      if (e.target.closest('#instruction-header')) {
        if (dragged && isHeading(dragged)) {
          e.preventDefault()
          const moving = [dragged, ...draggedSectionRows]
          shiftLevels(moving, 2 - headingLevel(dragged))
          tbody.prepend(...moving)
        }
        finishDrag()
        return
      }
      e.preventDefault()
      const target = e.target.closest('tr')
      clearDropMark()
      if (!dragged || !target || target === dragged || draggedSectionRows.includes(target)) {
        finishDrag()
        return
      }
      if (isHeading(dragged)) {
        /* dropped on a heading: the section becomes its last child */
        const moving = [dragged, ...draggedSectionRows]
        shiftLevels(moving, headingLevel(target) + 1 - headingLevel(dragged))
        sectionEndAfter(target).before(...moving)
      } else if (isHeading(target)) {
        /* dropped on a section heading: move to the end of that section */
        tbody.insertBefore(dragged, sectionEndAfter(target))
      } else {
        /* dropped on a row: insert right after it */
        target.after(dragged)
      }
      finishDrag()
    })
  }

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
  /* Large two-pane editor with three modes (options of openEditModal):
     'edit' — row pencil button: main variant + alternatives from the
     VARIANTS demo data below; 'create' — toolbar «+ New instruction»:
     empty name, one Main variant with empty text; 'custom' — per-project
     cell flows: text only, no variant list. Cancel/Esc with unsaved
     changes opens #unsaved-dialog (UI-010). */

  /* Alternative variants per instruction (demo data); the editor adds the
     main variant ("Main") on top of these when opening a row */
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

  /* Editor element refs (dialog chrome is grabbed at the top of the file) */
  const editForm = document.getElementById('edit-form')
  const ieBodyEl = document.getElementById('ie-body')
  const ieVariantList = document.getElementById('ie-variant-list')
  const ieAddCopy = document.getElementById('ie-add-copy')
  const ieAddEmpty = document.getElementById('ie-add-empty')
  const ieFields = document.getElementById('ie-fields')
  const ieName = document.getElementById('ie-name')
  const ieNameError = document.getElementById('ie-name-error')
  const ieVName = document.getElementById('ie-vname')
  const ieVNameError = document.getElementById('ie-vname-error')
  const ieTablist = document.getElementById('ie-tabs')
  const ieTabEdit = document.getElementById('ie-tab-edit')
  const ieTabPreview = document.getElementById('ie-tab-preview')
  const iePreview = document.getElementById('ie-preview')
  const ieWarningsList = document.getElementById('ie-warnings-list')
  const ieNoWarnings = document.getElementById('ie-no-warnings')
  const ieCharCount = document.getElementById('ie-char-count')
  const ieMarkerError = document.getElementById('ie-marker-error')
  const ieHistoryBtn = document.getElementById('ie-history-btn')
  const ieCancelBtn = document.getElementById('ie-cancel-btn')
  const unsavedDialog = document.getElementById('unsaved-dialog')
  const newInstructionBtn = document.getElementById('new-instruction-btn')

  const ieTabs = [ieTabEdit, ieTabPreview]

  const TEXT_LIMIT_LABEL = '262,144' // DB-006
  /* DB-016: a line equal to either marker rejects saving */
  const RESERVED_MARKERS = ['<!-- EUNOMIASYNC:BEGIN -->', '<!-- EUNOMIASYNC:END -->']

  /* Live state of the open editor; kept until the next open so the
     'close' handlers can still read it */
  let ieState = null
  let pendingVariantDelete = null

  function reservedMarkerLine(text) {
    return RESERVED_MARKERS.find((m) => text.split('\n').some((line) => line.trim() === m))
  }

  /* Main instruction name: 1–200 characters, unique among visible
     instructions case-insensitively (§6.9) — the edited row is excluded */
  function duplicateInstructionName(name, editedRow) {
    const lower = name.toLowerCase()
    return rows().some(
      (tr) =>
        tr !== editedRow &&
        !isHeading(tr) &&
        !tr.classList.contains('row-hidden') &&
        instructionName(tr) === lower,
    )
  }

  /* ---------- Tiny Markdown preview (D-15): sanitized subset --------------- */
  /* Hand-rolled renderer: everything is HTML-escaped first (escapeHtml),
     then only the renderer's own tags are added — headings ##…######,
     bold, italic, inline code, fenced code blocks and simple lists.
     Raw user HTML is never injected. */

  function mdInline(escaped) {
    const codes = []
    let t = escaped.replace(/`([^`]+)`/g, (_, code) => {
      codes.push(code)
      return `\u0000${codes.length - 1}\u0000`
    })
    t = t.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    t = t.replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>')
    t = t.replace(/(^|[\s(])_([^_\s][^_]*)_/g, '$1<em>$2</em>')
    t = t.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[Number(i)]}</code>`)
    return t
  }

  function renderMarkdown(src) {
    const lines = escapeHtml(src.replace(/\r\n/g, '\n')).split('\n')
    const out = []
    let inCode = false
    let list = null
    const closeList = () => {
      if (list) {
        out.push(`</${list}>`)
        list = null
      }
    }
    for (const line of lines) {
      const fence = line.match(/^\s*(`{3,}|~{3,})\s*([A-Za-z0-9_-]+)?\s*$/)
      if (fence) {
        if (!inCode) {
          closeList()
          out.push('<pre><code>')
          inCode = true
        } else {
          inCode = false
          out.push('</code></pre>')
        }
        continue
      }
      if (inCode) {
        out.push(`${line}\n`)
        continue
      }
      const heading = line.match(/^(#{2,6})\s+(.+)$/)
      if (heading) {
        closeList()
        out.push(`<h${heading[1].length}>${mdInline(heading[2])}</h${heading[1].length}>`)
        continue
      }
      const ul = line.match(/^\s*[-*]\s+(.+)$/)
      if (ul) {
        if (list !== 'ul') {
          closeList()
          out.push('<ul>')
          list = 'ul'
        }
        out.push(`<li>${mdInline(ul[1])}</li>`)
        continue
      }
      const ol = line.match(/^\s*\d+[.)]\s+(.+)$/)
      if (ol) {
        if (list !== 'ol') {
          closeList()
          out.push('<ol>')
          list = 'ol'
        }
        out.push(`<li>${mdInline(ol[1])}</li>`)
        continue
      }
      closeList()
      if (line.trim() !== '') out.push(`<p>${mdInline(line)}</p>`)
    }
    if (inCode) out.push('</code></pre>')
    closeList()
    return out.join('\n')
  }

  /* D-15: warnings never block saving */
  function mdWarnings(src) {
    const warnings = []
    let inCode = false
    let fenceChar = ''
    for (const raw of src.replace(/\r\n/g, '\n').split('\n')) {
      const fence = raw.match(/^\s*(`{3,}|~{3,})/)
      if (fence) {
        const ch = fence[1][0]
        if (!inCode) {
          inCode = true
          fenceChar = ch
        } else if (ch === fenceChar) {
          inCode = false
          fenceChar = ''
        }
        continue
      }
      if (!inCode && (raw.match(/`/g) ?? []).length % 2 === 1) {
        warnings.push('Unclosed inline code span — an odd number of backticks on the line.')
      }
    }
    if (inCode) warnings.push('Unclosed code fence — the fenced block is never closed (D-15).')
    return [...new Set(warnings)]
  }

  /* ---------- Editor state: dirty tracking, panes, variant list ------------ */

  function ieSnapshot() {
    return JSON.stringify({
      name: ieState.mode === 'custom' ? '' : ieName.value,
      variants: ieState.variants.map((v) => [v.name, v.text, v.main]),
    })
  }

  function ieIsDirty() {
    return ieState !== null && ieSnapshot() !== ieState.initial
  }

  function previewActive() {
    return ieTabPreview.getAttribute('aria-selected') === 'true'
  }

  function updateCharCount() {
    ieCharCount.textContent = `${editText.value.length.toLocaleString('en-US')} / ${TEXT_LIMIT_LABEL}`
  }

  function validateMarkerLive() {
    const marker = reservedMarkerLine(ieState.variants[ieState.selected].text)
    ieMarkerError.hidden = !marker
    ieMarkerError.textContent = marker
      ? `The line ${marker} is reserved for the sync area and cannot be used in the text (DB-016).`
      : ''
    editText.setAttribute('aria-invalid', marker ? 'true' : 'false')
  }

  function renderPreviewPane() {
    const src = ieState.variants[ieState.selected].text
    iePreview.innerHTML = renderMarkdown(src)
    const warnings = mdWarnings(src)
    ieWarningsList.hidden = warnings.length === 0
    ieNoWarnings.hidden = warnings.length !== 0
    ieWarningsList.replaceChildren(
      ...warnings.map((w) => {
        const li = document.createElement('li')
        li.textContent = w
        return li
      }),
    )
  }

  /* Edit/Preview tabs: same roving-focus pattern as the project settings */
  function selectIeTab(tab, moveFocus) {
    for (const t of ieTabs) {
      const selected = t === tab
      t.setAttribute('aria-selected', String(selected))
      t.tabIndex = selected ? 0 : -1
      document.getElementById(t.getAttribute('aria-controls')).hidden = !selected
    }
    if (moveFocus) tab.focus()
  }

  /* Variant list items: name + Main badge + dashed mark for empty ones;
     actions are "Make main" and "Delete" (LIB-021, LIB-023…025) */
  function renderVariantList() {
    if (!ieState || ieState.mode === 'custom') return
    ieVariantList.replaceChildren(
      ...ieState.variants.map((v, i) => {
        const rowEl = document.createElement('div')
        rowEl.className = 'ie-variant-row'
        rowEl.dataset.idx = String(i)

        const item = document.createElement('button')
        item.type = 'button'
        item.className = v.text === '' ? 'ie-variant-item ie-variant-empty' : 'ie-variant-item'
        item.setAttribute('aria-pressed', String(i === ieState.selected))
        item.title =
          v.text === '' ? 'Empty variant — outputs no text, can be assigned (D-13)' : v.name
        if (v.text === '') {
          const mark = document.createElement('span')
          mark.className = 'empty-mark'
          mark.setAttribute('aria-hidden', 'true')
          item.append(mark)
        }
        const label = document.createElement('span')
        label.className = 'ie-variant-label'
        label.textContent = v.name
        item.append(label)
        if (v.main) {
          const badge = document.createElement('span')
          badge.className = 'ie-main-badge'
          badge.textContent = 'Main'
          item.append(badge)
        }

        const mainBtn = document.createElement('button')
        mainBtn.type = 'button'
        mainBtn.className = 'cell-icon cell-icon--main'
        mainBtn.innerHTML = ICONS.main
        mainBtn.title = v.main ? 'Already the main variant' : `Make main — ${v.name}`
        mainBtn.setAttribute('aria-label', mainBtn.title)
        mainBtn.disabled = v.main

        const delBtn = document.createElement('button')
        delBtn.type = 'button'
        delBtn.className = 'cell-icon cell-icon--delete'
        delBtn.innerHTML = ICONS.delete
        if (ieState.variants.length === 1) {
          delBtn.disabled = true
          delBtn.title = 'The last variant cannot be deleted (LIB-025)'
        } else if (v.main) {
          delBtn.disabled = true
          delBtn.title =
            'Make another variant main first — the main variant cannot be deleted (LIB-025)'
        } else {
          delBtn.title = `Delete variant — ${v.name}`
        }
        delBtn.setAttribute('aria-label', delBtn.title)

        rowEl.append(item, mainBtn, delBtn)
        return rowEl
      }),
    )
  }

  function loadSelectedVariant() {
    const v = ieState.variants[ieState.selected]
    ieVName.value = v.name
    editText.value = v.text
    editText.placeholder =
      ieState.mode === 'custom'
        ? ieState.placeholder
        : v.text === ''
          ? v.main
            ? 'No text yet — write the main variant here.'
            : 'No text yet — write the variant text here.'
          : 'Instruction text (Markdown)…'
    updateCharCount()
    validateMarkerLive()
    setFieldError(ieVName, ieVNameError, '')
    if (previewActive()) renderPreviewPane()
  }

  function selectVariant(i) {
    if (i === ieState.selected) return
    ieState.selected = i
    loadSelectedVariant()
    renderVariantList()
  }

  function makeMain(i) {
    ieState.variants.forEach((v, j) => (v.main = j === i))
    renderVariantList()
    ieVariantList
      .querySelector(`.ie-variant-row[data-idx='${i}'] .cell-icon--main`)
      ?.focus()
  }

  function requestDeleteVariant(i) {
    const v = ieState.variants[i]
    pendingVariantDelete = i
    confirmTitle.textContent = 'Delete variant'
    confirmText.textContent =
      `Delete the variant "${v.name}"? Its assignments move to the main variant ` +
      '(the main one is the default replacement — LIB-023…025).'
    confirmDialog.showModal()
  }

  /* LIB-017: "Add variant" offers a copy of the main one or an empty one */
  function freeVariantName() {
    const taken = new Set(ieState.variants.map((v) => v.name.trim().toLowerCase()))
    for (let n = 1; ; n += 1) {
      if (!taken.has(`variant ${n}`)) return `Variant ${n}`
    }
  }

  function addVariant(copyOfMain) {
    const main = ieState.variants.find((v) => v.main) ?? ieState.variants[0]
    ieState.variants.push({
      name: freeVariantName(),
      text: copyOfMain ? main.text : '',
      main: false,
    })
    ieState.selected = ieState.variants.length - 1
    loadSelectedVariant()
    renderVariantList()
    ieVName.focus()
    ieVName.select()
  }

  /* ---------- Editor validation and save ------------------------------------ */

  function validateEditor() {
    let ok = true
    if (ieState.mode !== 'custom') {
      const name = ieName.value.trim()
      if (name.length < 1 || name.length > 200) {
        setFieldError(ieName, ieNameError, 'Name is required — 1–200 characters, one line (§6.9).')
        ok = false
      } else if (duplicateInstructionName(name, ieState.row)) {
        setFieldError(ieName, ieNameError, 'This name is already used by another visible instruction.')
        ok = false
      } else {
        setFieldError(ieName, ieNameError, '')
      }
      /* Variant names: 1–100 characters, unique within the instruction */
      let badIdx = -1
      let badMsg = ''
      const seen = new Map()
      ieState.variants.forEach((v, i) => {
        const vn = v.name.trim()
        if (vn.length < 1 || vn.length > 100) {
          if (badIdx === -1) {
            badIdx = i
            badMsg = 'Variant name is required — 1–100 characters (§6.9).'
          }
        } else if (seen.has(vn.toLowerCase())) {
          if (badIdx === -1) {
            badIdx = i
            badMsg = 'Variant names must be unique within the instruction (case-insensitive).'
          }
        } else {
          seen.set(vn.toLowerCase(), i)
        }
      })
      if (badIdx !== -1) {
        selectVariant(badIdx)
        setFieldError(ieVName, ieVNameError, badMsg)
        ok = false
      }
    }
    /* Reserved markers reject saving on every variant text (DB-016) */
    let markerIdx = -1
    ieState.variants.forEach((v, i) => {
      if (markerIdx === -1 && reservedMarkerLine(v.text)) markerIdx = i
    })
    if (markerIdx !== -1) {
      selectVariant(markerIdx)
      validateMarkerLive()
      ok = false
    }
    return ok
  }

  /* ---------- Open in one of the three modes -------------------------------- */

  function openEditModal({
    mode = 'edit',
    row = null,
    title,
    name = '',
    text = '',
    placeholder = '',
    variants = [],
  }) {
    /* The main variant always heads the list; alternatives follow it */
    const list = [
      { name: 'Main', text, main: true },
      ...variants.map((v) => ({ name: v.name, text: v.text, main: false })),
    ]
    const resolvedTitle =
      title ?? (mode === 'create' ? 'New instruction' : `Edit instruction — ${name}`)
    ieState = {
      mode,
      row,
      rowName: name,
      placeholder,
      historyName:
        mode === 'custom'
          ? (title ?? '').replace(/^Custom instruction — /, '')
          : resolvedTitle.replace(/^Edit instruction — /, ''),
      variants: list,
      selected: 0,
      initial: '',
    }
    editTitle.textContent = resolvedTitle
    ieBodyEl.classList.toggle('ie-custom', mode === 'custom')
    ieName.value = mode === 'custom' ? '' : name
    setFieldError(ieName, ieNameError, '')
    ieMarkerError.hidden = true
    selectIeTab(ieTabEdit, false)
    loadSelectedVariant()
    renderVariantList()
    ieState.initial = ieSnapshot()
    editDialog.showModal()
    ;(mode === 'custom' ? editText : ieName).focus()
  }

  /* ---------- Editor event wiring ------------------------------------------- */

  /* Save: validate on the Save button click (preventing it keeps the
     dialog open with inline errors — same pattern as project settings) */
  editForm.querySelector('button[value="save"]').addEventListener('click', (e) => {
    if (!validateEditor()) e.preventDefault()
  })

  /* Enter in a text field saves — the implicit submitter would be Cancel */
  editForm.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || e.target.tagName !== 'INPUT') return
    e.preventDefault()
    if (validateEditor()) editDialog.close('save')
  })

  /* UI-010: Cancel and Esc with unsaved changes ask for confirmation */
  function requestCloseEditor() {
    if (ieIsDirty()) unsavedDialog.showModal()
    else editDialog.close('cancel')
  }

  ieCancelBtn.addEventListener('click', (e) => {
    e.preventDefault()
    requestCloseEditor()
  })

  editDialog.addEventListener('cancel', (e) => {
    if (!ieIsDirty()) return // clean: let the default Esc-close happen
    e.preventDefault()
    unsavedDialog.showModal()
  })

  unsavedDialog.addEventListener('close', () => {
    if (unsavedDialog.returnValue === 'discard' && editDialog.open) {
      editDialog.close('cancel')
    }
  })

  /* History (UI-003): opens the revisions panel (LIB-026…027, D-16) */
  ieHistoryBtn.addEventListener('click', () => {
    openHistory(ieState?.historyName ?? '')
  })

  /* Toolbar «+ New instruction» opens the editor in CREATE mode (LIB-002) */
  newInstructionBtn.addEventListener('click', () => openEditModal({ mode: 'create' }))

  /* Variant list: one delegated click handler for select / make main /
     delete; the disabled buttons never reach it */
  ieVariantList.addEventListener('click', (e) => {
    const rowEl = e.target.closest('.ie-variant-row')
    if (!rowEl || !ieState) return
    const i = Number(rowEl.dataset.idx)
    if (e.target.closest('.cell-icon--main')) {
      makeMain(i)
      return
    }
    if (e.target.closest('.cell-icon--delete')) {
      requestDeleteVariant(i)
      return
    }
    if (e.target.closest('.ie-variant-item')) selectVariant(i)
  })

  ieAddCopy.addEventListener('click', () => addVariant(true))
  ieAddEmpty.addEventListener('click', () => addVariant(false))

  for (const tab of ieTabs) {
    tab.addEventListener('click', () => {
      selectIeTab(tab, false)
      if (tab === ieTabPreview && ieState) renderPreviewPane()
    })
  }

  ieTablist.addEventListener('keydown', (e) => {
    const current = ieTabs.indexOf(document.activeElement)
    if (current === -1) return
    let next = null
    if (e.key === 'ArrowRight') next = (current + 1) % ieTabs.length
    else if (e.key === 'ArrowLeft') next = (current + ieTabs.length - 1) % ieTabs.length
    else if (e.key === 'Home') next = 0
    else if (e.key === 'End') next = ieTabs.length - 1
    if (next === null) return
    e.preventDefault()
    selectIeTab(ieTabs[next], true)
  })

  ieName.addEventListener('input', () => setFieldError(ieName, ieNameError, ''))

  ieVName.addEventListener('input', () => {
    if (!ieState) return
    ieState.variants[ieState.selected].name = ieVName.value
    setFieldError(ieVName, ieVNameError, '')
    renderVariantList()
  })

  editText.addEventListener('input', () => {
    if (!ieState) return
    const v = ieState.variants[ieState.selected]
    v.text = editText.value.replace(/\r\n/g, '\n') // CRLF -> LF on save (§6.9)
    updateCharCount()
    validateMarkerLive()
    if (previewActive()) renderPreviewPane()
  })

  /* Saving in EDIT mode applies the name to the row (the rest of the demo
     state stays in-memory; VARIANTS is re-keyed so the cell selects keep
     finding their texts) */
  editDialog.addEventListener('close', () => {
    const st = ieState
    if (!st || st.mode !== 'edit' || editDialog.returnValue !== 'save') return
    const name = ieName.value.trim()
    if (!st.row || name === st.rowName) return
    st.row.querySelector('.name').textContent = name
    st.row.querySelector('.edit-btn')?.setAttribute('aria-label', `Edit instruction: ${name}`)
    st.row.querySelector('.row-sel')?.setAttribute('aria-label', `Select row: ${name}`)
    for (const sel of st.row.querySelectorAll('.variant-select')) {
      sel.setAttribute('aria-label', `Assigned text — ${name}`)
    }
    const mapped = VARIANTS[st.rowName]
    if (mapped) {
      delete VARIANTS[st.rowName]
      VARIANTS[name] = mapped
    }
  })

  tbody.addEventListener('click', (e) => {
    const btn = e.target.closest('.edit-btn')
    if (!btn) return
    const row = btn.closest('tr')
    const name = instructionDisplay(row)
    const mainText = instructionText(row)

    /* The variant list shows the main variant first, then every alternative
       the instruction has: mapped ones explicitly; single-variant rows get
       their "Variant 1" synthesized from the instruction text; badge-less
       rows have no alternatives at all. "Variant 1" is the default wording
       of the main text: nudge it a bit so it visibly differs from the main
       variant */
    const variantCount = Number(row.querySelector('.chip')?.textContent ?? 0)
    const alternatives = (
      VARIANTS[name] ?? (variantCount >= 1 ? [{ name: 'Variant 1', text: mainText }] : [])
    ).map((v) =>
      v.name === 'Variant 1' && v.text === mainText
        ? { name: v.name, text: v.text + ' Slightly reworded version of the main text.' }
        : v,
    )
    openEditModal({
      mode: 'edit',
      row,
      name,
      text: mainText,
      variants: alternatives,
    })
  })

  /* ---------- History modal (LIB-026, D-16) --------------------------------- */
  /* Revisions panel of an instruction: the instruction itself and every
     variant are separate entities, each with its own rev_no sequence
     (§6.2). Newest first, an op badge per row (NFR-020: the operation is
     always readable as text) and a <details> disclosure showing what the
     snapshot holds — name + state for the instruction, name + text for a
     variant. A deleted variant can be restored (LIB-027): the row flips
     to a restore badge and, while the editor is open for the same
     instruction, the variant is appended back into its in-memory list.
     Read-only apart from restore; up to 200 versions per entity are kept,
     the latest one never deleted (D-16). */

  const histDialog = document.getElementById('history-dialog')
  const histTitle = document.getElementById('hist-title')
  const histTable = document.querySelector('#history-dialog .hist-table')
  const histRows = document.getElementById('hist-rows')
  const histEmpty = document.getElementById('hist-empty')
  const restoreDialog = document.getElementById('restore-dialog')
  const restoreText = document.getElementById('restore-text')

  /* Demo clock: "now" matches the sync bar and the EunomiaSync column
     (19 Nov 2026, 14:32); stamps are rendered in the fixed UTC+3 the demo
     uses, regardless of the viewer's timezone */
  const HIST_NOW_MS = Date.parse('2026-11-19T14:32:00+03:00')

  function histStamp(minutesBack) {
    const d = new Date(HIST_NOW_MS - minutesBack * 60000 + 180 * 60000)
    const hh = String(d.getUTCHours()).padStart(2, '0')
    const mm = String(d.getUTCMinutes()).padStart(2, '0')
    return `${d.getUTCDate()} ${MONTHS_SHORT[d.getUTCMonth()]} ${d.getUTCFullYear()}, ${hh}:${mm} (UTC+3)`
  }

  /* Live state of the open history panel; kept until the next open */
  let histState = null

  /* Demo revisions for one instruction, derived from the row and the
     VARIANTS map the same way the editor derives its variant list; the
     latest snapshot of every entity matches the current state. Offsets
     are minutes back from the demo "now" (larger = older), and rev_no is
     unique per entity. Ops: create / update / hide / restore / delete. */
  function buildDemoRevisions(name, row) {
    const mainText = instructionText(row)
    const chipCount = Number(row.querySelector('.chip')?.textContent ?? 0)
    const alts = (
      VARIANTS[name] ?? (chipCount >= 1 ? [{ name: 'Variant 1', text: mainText }] : [])
    ).map((v) =>
      v.name === 'Variant 1' && v.text === mainText
        ? { name: v.name, text: v.text + ' Slightly reworded version of the main text.' }
        : v,
    )
    const hidden = row.classList.contains('row-hidden')
    const revs = []
    const push = (entityType, entityName, op, revNo, minutesBack, snapshot) =>
      revs.push({ entityType, entityName, op, revNo, minutesBack, snapshot })

    /* Instruction entity: created active; a hidden row also carries a hide */
    push('instruction', name, 'create', 1, 8940, { name, state: 'active' })
    if (hidden) push('instruction', name, 'hide', 2, 240, { name, state: 'hidden' })

    /* Main variant: a first-draft wording, later updated to the current text */
    push('variant', 'Main', 'create', 1, 8870, {
      name: 'Main',
      text: mainText ? `${mainText} (first draft)` : '',
    })
    if (mainText) push('variant', 'Main', 'update', 2, 2980, { name: 'Main', text: mainText })

    /* A former variant, deleted afterwards — its delete row carries
       "Restore variant" (LIB-027) */
    const oldWordingText = mainText
      ? `${mainText} (superseded wording)`
      : 'Superseded wording of the instruction text.'
    push('variant', 'Old wording', 'create', 1, 8800, {
      name: 'Old wording',
      text: oldWordingText,
    })
    push('variant', 'Old wording', 'delete', 2, 5700, {
      name: 'Old wording',
      text: oldWordingText,
    })

    /* Alternatives: the first gets two updates, the rest one; the last one
       also goes through a hide + restore pair */
    alts.forEach((v, i) => {
      let no = 1
      push('variant', v.name, 'create', no++, i === 0 ? 5800 : 4320, {
        name: v.name,
        text: `${v.text} (first draft)`,
      })
      const updateOffsets = i === 0 ? [4400, 3000] : [2940]
      for (const off of updateOffsets) {
        const text = off === updateOffsets[updateOffsets.length - 1] ? v.text : `${v.text} (reworded)`
        push('variant', v.name, 'update', no++, off, { name: v.name, text })
      }
      if (i === alts.length - 1) {
        push('variant', v.name, 'hide', no++, 1500, { name: v.name, text: v.text })
        push('variant', v.name, 'restore', no++, 1440, { name: v.name, text: v.text })
      }
    })

    /* Newest first: the smaller the offset, the more recent the revision */
    revs.sort((a, b) => a.minutesBack - b.minutesBack)
    return revs
  }

  function histSnapshotHtml(rev) {
    const s = rev.snapshot
    const nameRow = `<div class="hist-snap-row"><span class="hist-snap-key">Name</span><span>${escapeHtml(s.name)}</span></div>`
    const body =
      rev.entityType === 'instruction'
        ? `${nameRow}<div class="hist-snap-row"><span class="hist-snap-key">State</span><span>${s.state}</span></div>`
        : `${nameRow}<pre class="hist-snap-text">${escapeHtml(s.text)}</pre>`
    return `<details class="hist-snap"><summary>Snapshot</summary><div class="hist-snap-body">${body}</div></details>`
  }

  function renderRevisions() {
    const revs = histState.revs
    histEmpty.hidden = revs.length !== 0
    histTable.hidden = revs.length === 0
    histRows.innerHTML = revs
      .map((rev, i) => {
        const entity =
          rev.entityType === 'instruction'
            ? 'Instruction'
            : `${escapeHtml(rev.entityName)}${rev.restored ? ' <span class="chip hist-restored">Restored</span>' : ''}`
        const restoreBtn =
          rev.op === 'delete'
            ? `<button type="button" class="btn btn-ghost hist-restore-btn" data-hist-restore="${i}">Restore variant</button>`
            : ''
        return (
          '<tr>' +
          `<td class="hist-no">#${rev.revNo}</td>` +
          `<td class="hist-when">${histStamp(rev.minutesBack)}</td>` +
          `<td class="hist-op-cell"><span class="chip hist-op hist-op--${rev.op}">${rev.op}</span></td>` +
          `<td class="hist-entity">${entity}${restoreBtn}</td>` +
          `<td class="hist-snap-cell">${histSnapshotHtml(rev)}</td>` +
          '</tr>'
        )
      })
      .join('')
  }

  function openHistory(name) {
    const wanted = (name ?? '').trim().toLowerCase()
    const row = rows().find((tr) => !isHeading(tr) && instructionName(tr) === wanted)
    histState = {
      name: name ?? '',
      revs: row ? buildDemoRevisions(name, row) : [],
      pendingRestore: null,
    }
    histTitle.textContent = `History — ${name || 'instruction'}`
    renderRevisions()
    histDialog.showModal()
  }

  /* Restore variant (LIB-027): confirm first, then flip the delete row to
     a restore badge; no new revisions appear (demo feedback only) */
  histRows.addEventListener('click', (e) => {
    const btn = e.target.closest('[data-hist-restore]')
    if (!btn || !histState) return
    const rev = histState.revs[Number(btn.dataset.histRestore)]
    if (!rev || rev.op !== 'delete') return
    histState.pendingRestore = rev
    restoreText.textContent =
      `Restore the variant "${rev.snapshot.name}"? It comes back with its name and text; ` +
      'no version is erased (LIB-027).'
    restoreDialog.showModal()
  })

  restoreDialog.addEventListener('close', () => {
    const st = histState
    const rev = st?.pendingRestore
    if (st) st.pendingRestore = null
    if (!rev || restoreDialog.returnValue !== 'restore') return
    rev.op = 'restore'
    rev.restored = true
    renderRevisions()
    /* With the editor open for the same instruction, the variant also
       returns to its in-memory variant list (as a real change: the editor
       becomes dirty and the usual unsaved-changes guard applies) */
    if (
      ieState &&
      ieState.mode !== 'custom' &&
      editDialog.open &&
      ieState.historyName === st.name &&
      !ieState.variants.some((v) => v.name.toLowerCase() === rev.snapshot.name.toLowerCase())
    ) {
      ieState.variants.push({ name: rev.snapshot.name, text: rev.snapshot.text, main: false })
      renderVariantList()
    }
  })

  /* ---------- Per-project custom instructions (cell icons + flows) -------- */

  let pendingCustomCell = null
  let pendingDeleteCell = null
  const customTexts = new WeakMap()

  /* Solid icons (Material-style paths), colored via currentColor */
  const ICONS = {
    add: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 3h2v8h8v2h-8v8h-2v-8H3v-2h8V3z"/></svg>',
    edit: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.03a.996.996 0 0 0 0-1.41l-2.34-2.34a.996.996 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>',
    delete:
      '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>',
    main: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 17.27 18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/></svg>',
    /* Toolbar buttons (Activity, Settings): Material-style solid glyphs,
       14px, filled with currentColor like the cell icons */
    activity:
      '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"/></svg>',
    settings:
      '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>',
  }

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
    if (columnUnavailable) {
      box?.remove()
      return
    }
    /* chip-less cells keep a bare checkbox in the td: give them the same
       flex wrapper so the icon aligns with it */
    let wrap = td.querySelector('.cell-assign')
    if (!wrap) {
      wrap = document.createElement('span')
      wrap.className = 'cell-assign'
      input.before(wrap)
      wrap.append(input)
    }
    if (!box) {
      box = document.createElement('span')
      box.className = 'cell-icons'
      wrap.append(box)
    }
    const mk = (kind, title) => {
      const b = document.createElement('button')
      b.type = 'button'
      b.className = `cell-icon cell-icon--${kind}`
      b.title = title
      b.setAttribute('aria-label', title)
      b.innerHTML = ICONS[kind]
      return b
    }
    box.replaceChildren()
    if (isCustom) {
      const edit = mk('edit', `Edit the custom instruction — ${instr} — ${proj}`)
      edit.addEventListener('click', () => {
        pendingCustomCell = td
        openEditModal({
          mode: 'custom',
          title: `Custom instruction — ${instr} — ${proj}`,
          text: customTexts.get(td) ?? '',
          placeholder: 'Write the custom instruction text…',
        })
      })
      const del = mk('delete', `Delete the custom instruction — ${instr} — ${proj}`)
      del.addEventListener('click', () => {
        pendingDeleteCell = td
        confirmTitle.textContent = 'Delete custom instruction'
        confirmText.textContent = `Delete the custom instruction for ${instr} — ${proj}? The cell returns to the selected option of the list.`
        confirmDialog.showModal()
      })
      box.append(edit, del)
    } else {
      const add = mk('add', `Create a custom instruction — ${instr} — ${proj}`)
      add.addEventListener('click', () => {
        pendingCustomCell = td
        openEditModal({
          mode: 'custom',
          title: `Custom instruction — ${instr} — ${proj}`,
          text: optionText(row, select ? select.value : 'Default'),
          placeholder: 'Write the custom instruction text…',
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

  /* Variant deletion confirmed from the editor's confirm dialog (LIB-023…025):
     the last variant and the main one never reach this — their buttons are
     disabled in the list */
  confirmDialog.addEventListener('close', () => {
    const i = pendingVariantDelete
    pendingVariantDelete = null
    if (i === null || !ieState || confirmDialog.returnValue !== 'delete') return
    ieState.variants.splice(i, 1)
    ieState.selected = Math.min(ieState.selected, ieState.variants.length - 1)
    loadSelectedVariant()
    renderVariantList()
  })

  /* ---------- Project settings modal (PRJ-008…012) ------------------------- */
  /* Opened by the pencil button in a project column header. Fields are
     prefilled from the demo data below; Save updates the column header
     live. Archive uses type-to-confirm (PRJ-013) and stays non-destructive. */

  /* Fixed 12-color palette (DB-040): [hex, accessible name] */
  const PS_PALETTE = [
    ['#ef4444', 'Red'],
    ['#f97316', 'Orange'],
    ['#f59e0b', 'Amber'],
    ['#84cc16', 'Lime'],
    ['#10b981', 'Green'],
    ['#14b8a6', 'Teal'],
    ['#06b6d4', 'Cyan'],
    ['#3b82f6', 'Blue'],
    ['#6366f1', 'Indigo'],
    ['#8b5cf6', 'Violet'],
    ['#ec4899', 'Pink'],
    ['#64748b', 'Slate'],
  ]

  /* Demo project data, keyed by name; colors match the column headers in
     the markup, sync times match their `title` tooltips. `editBtn` is
     attached in the init section. */
  const PROJECTS = new Map(
    [
      {
        name: 'EunomiaSync',
        label: 'ES',
        color: '#10b981',
        description: 'The app itself, syncing these instructions into its own AGENTS.md.',
        path: '/home/brooks/projects/business/EunomiaSync',
        addedVia: 'Manual',
        scheme: 'A',
        baseFile: 'AGENTS.md',
        extraLocal: true,
        extraLocalPos: 'end',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '19 Nov 2026, 14:32 (UTC+3)',
        runs: 47,
      },
      {
        name: 'yepbro/site',
        label: 'SITE',
        color: '#3b82f6',
        description: 'Marketing site repo; agent rules live in the marked area of AGENTS.md.',
        path: '/home/brooks/projects/yepbro/site',
        addedVia: 'Found by scan',
        scheme: 'B',
        baseFile: 'AGENTS.md',
        extraLocal: false,
        extraLocalPos: 'end',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '19 Nov 2026, 09:14 (UTC+3)',
        runs: 12,
      },
      {
        name: 'quotameter',
        label: 'QM',
        color: '#8b5cf6',
        description: 'Metering dashboard; the target file is CLAUDE.md here.',
        path: '/home/brooks/projects/business/quotameter',
        addedVia: 'Manual',
        scheme: 'A',
        baseFile: 'CLAUDE.md',
        extraLocal: false,
        extraLocalPos: 'end',
        extraProject: true,
        extraProjectPos: 'end',
        syncedAt: '12 Nov 2026, 09:10 (UTC+3)',
        runs: 8,
      },
      {
        name: 'task-id-mcp',
        label: 'TIM',
        color: '#f59e0b',
        description: 'Task numbering MCP service (Docker, autostart).',
        path: '/home/brooks/projects/utils/task-id-mcp',
        addedVia: 'Manual',
        scheme: 'A',
        baseFile: 'AGENTS.md',
        extraLocal: true,
        extraLocalPos: 'end',
        extraProject: true,
        extraProjectPos: 'start',
        syncedAt: '18 Nov 2026, 20:41 (UTC+3)',
        runs: 31,
      },
      {
        name: 'utils/deja-vu',
        label: 'DV',
        color: '#14b8a6',
        description: 'Past-session memory search; the folder is currently unavailable.',
        path: '/home/brooks/projects/utils/deja-vu',
        addedVia: 'Found by scan',
        scheme: 'A',
        baseFile: 'AGENTS.md',
        extraLocal: false,
        extraLocalPos: 'end',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '3 Nov 2026, 21:05 (UTC+3)',
        runs: 5,
      },
      {
        name: 'docktree',
        label: 'DT',
        color: '#ec4899',
        description: 'Container layout helper; instructions go to a separate project file.',
        path: '/home/brooks/projects/tools/docktree',
        addedVia: 'Manual',
        scheme: 'C',
        baseFile: 'AGENTS.md',
        extraLocal: false,
        extraLocalPos: 'end',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '17 Nov 2026, 11:02 (UTC+3)',
        runs: 22,
      },
      {
        name: 'moli',
        label: 'MOLI',
        color: '#6366f1',
        description: 'Small gallery app; local overrides are included at the start of the file.',
        path: '/home/brooks/projects/apps/moli',
        addedVia: 'Manual',
        scheme: 'A',
        baseFile: 'AGENTS.md',
        extraLocal: true,
        extraLocalPos: 'start',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '15 Nov 2026, 18:27 (UTC+3)',
        runs: 14,
      },
      {
        name: 'sandbox-cf',
        label: 'SB',
        color: '#64748b',
        description: 'Cloudflare experiments sandbox; the target file is not created yet.',
        path: '/home/brooks/projects/labs/sandbox-cf',
        addedVia: 'Found by scan',
        scheme: 'B',
        baseFile: 'AGENTS.md',
        extraLocal: false,
        extraLocalPos: 'end',
        extraProject: false,
        extraProjectPos: 'end',
        syncedAt: '10 Nov 2026, 08:55 (UTC+3)',
        runs: 3,
      },
    ].map((p) => [p.name, p]),
  )

  const psDialog = document.getElementById('project-settings-dialog')
  const psForm = document.getElementById('ps-form')
  const psTitle = document.getElementById('ps-title')
  const psTablist = document.getElementById('ps-tabs')
  const psTabGeneral = document.getElementById('ps-tab-general')
  const psTabBackups = document.getElementById('ps-tab-backups')
  const psName = document.getElementById('ps-name')
  const psNameError = document.getElementById('ps-name-error')
  const psLabel = document.getElementById('ps-label')
  const psLabelError = document.getElementById('ps-label-error')
  const psLabelPreview = document.getElementById('ps-label-preview')
  const psDescription = document.getElementById('ps-description')
  const psDescCount = document.getElementById('ps-desc-count')
  const psSwatches = document.getElementById('ps-swatches')
  const psPath = document.getElementById('ps-path')
  const psAddedVia = document.getElementById('ps-added-via')
  const psLastSync = document.getElementById('ps-last-sync')
  const psSchemes = document.getElementById('ps-schemes')
  const psBaseFile = document.getElementById('ps-base-file')
  const psExtraFiles = document.getElementById('ps-extra-files')
  const psExtraLocal = document.getElementById('ps-extra-local')
  const psExtraLocalPos = document.getElementById('ps-extra-local-pos')
  const psExtraProject = document.getElementById('ps-extra-project')
  const psExtraProjectPos = document.getElementById('ps-extra-project-pos')
  const psExtraLocalName = document.getElementById('ps-extra-local-name')
  const psExtraProjectName = document.getElementById('ps-extra-project-name')
  const psSchemeB = document.getElementById('ps-scheme-b')
  const psSchemeC = document.getElementById('ps-scheme-c')
  const psSnippet = document.getElementById('ps-snippet')
  const psBackupsBody = document.getElementById('ps-backups-body')
  const psArchiveBtn = document.getElementById('ps-archive-btn')
  const archiveDialog = document.getElementById('archive-dialog')
  const archiveText = document.getElementById('archive-text')
  const archiveInput = document.getElementById('archive-input')
  const archiveConfirmBtn = document.getElementById('archive-confirm-btn')

  const psTabs = [psTabGeneral, psTabBackups]
  let psCurrentName = ''

  /* Swatch radios are generated so the palette stays single-sourced */
  for (const [hex, colorName] of PS_PALETTE) {
    const label = document.createElement('label')
    label.className = 'ps-swatch'
    label.title = colorName
    const input = document.createElement('input')
    input.type = 'radio'
    input.name = 'ps-color'
    input.value = hex
    input.setAttribute('aria-label', colorName)
    const fill = document.createElement('span')
    fill.className = 'ps-swatch-fill'
    fill.style.setProperty('--c', hex)
    label.append(input, fill)
    psSwatches.append(label)
  }

  const psColorInput = () => psSwatches.querySelector('input:checked')
  const psSchemeInput = () => psSchemes.querySelector('input:checked')
  const stemOf = (fileName) => fileName.replace(/\.md$/, '')

  /* <STEM>.local.md / <STEM>.project.md follow the base file name (D-20);
     the scheme C snippet uses the D-21 wording */
  function updateDerivedNames() {
    const stem = stemOf(psBaseFile.value)
    psSnippet.textContent = `If ${stem}.project.md exists in the repository root, read it before starting work and follow its instructions.`
    psExtraLocalName.textContent = `${stem}.local.md`
    psExtraProjectName.textContent = `${stem}.project.md`
  }

  /* Which blocks are visible per scheme: extras for A, marker pair for B,
     snippet for C */
  function updateSchemeUI() {
    const scheme = psSchemeInput()?.value ?? 'A'
    psExtraFiles.hidden = scheme !== 'A'
    psSchemeB.hidden = scheme !== 'B'
    psSchemeC.hidden = scheme !== 'C'
  }

  function syncExtraRowUI() {
    psExtraLocalPos.disabled = !psExtraLocal.checked
    psExtraProjectPos.disabled = !psExtraProject.checked
  }

  function setFieldError(input, errorEl, message) {
    errorEl.hidden = !message
    errorEl.textContent = message
    input.setAttribute('aria-invalid', message ? 'true' : 'false')
  }

  /* §6.9: name 1–100 after trimming, label 1–6 visible characters (DB-042);
     description is capped by maxlength=1000 */
  function validatePsFields() {
    let ok = true
    const name = psName.value.trim()
    if (name.length < 1 || name.length > 100) {
      setFieldError(psName, psNameError, 'Name is required — 1–100 characters after trimming.')
      ok = false
    } else if (name !== psCurrentName && PROJECTS.has(name)) {
      /* Demo guard: the real app only warns about duplicates (§6.9) */
      setFieldError(psName, psNameError, 'This name is already used by another project.')
      ok = false
    } else {
      setFieldError(psName, psNameError, '')
    }
    const label = psLabel.value.trim()
    if (label.length < 1 || label.length > 6) {
      setFieldError(psLabel, psLabelError, 'Label is required — 1–6 visible characters (DB-042).')
      ok = false
    } else {
      setFieldError(psLabel, psLabelError, '')
    }
    return ok
  }

  const MONTHS_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

  function shiftedDate(dateStr, daysBack) {
    const d = new Date(dateStr)
    d.setDate(d.getDate() - daysBack)
    return `${d.getDate()} ${MONTHS_SHORT[d.getMonth()]} ${d.getFullYear()}`
  }

  /* BKP-004 demo rows: the three latest backups of the base file */
  function renderBackups(p) {
    const [date, timeTz] = p.syncedAt.split(', ')
    const m = timeTz.match(/^(\d{2}:\d{2}) (.+)$/)
    const time = m ? m[1] : timeTz
    const tz = m ? ` ${m[2]}` : ''
    const sizes = ['8.4 KB', '8.1 KB', '7.9 KB']
    psBackupsBody.innerHTML = [0, 1, 2]
      .map((i) => {
        const cells = [
          `${shiftedDate(date, i)}, ${time}${tz}`,
          p.baseFile,
          sizes[i],
          `#${Math.max(1, p.runs - i)}`,
        ]
        return (
          '<tr>' +
          cells
            .map(
              (cell, ci) =>
                `<td${ci === 1 ? ' class="ps-backup-file"' : ''}>${escapeHtml(cell)}</td>`,
            )
            .join('') +
          '</tr>'
        )
      })
      .join('')
  }

  function selectPsTab(tab, moveFocus) {
    for (const t of psTabs) {
      const selected = t === tab
      t.setAttribute('aria-selected', String(selected))
      t.tabIndex = selected ? 0 : -1
      const panel = document.getElementById(t.getAttribute('aria-controls'))
      if (panel) panel.hidden = !selected
    }
    if (moveFocus) tab.focus()
  }

  function openProjectSettings(name) {
    const p = PROJECTS.get(name)
    if (!p) return
    psCurrentName = name
    psTitle.textContent = `Project settings — ${name}`
    psName.value = p.name
    psLabel.value = p.label
    psLabelPreview.textContent = p.label || '—'
    psDescription.value = p.description ?? ''
    psDescCount.textContent = `${psDescription.value.length} / 1000`
    /* PRJ-010: the prototype preselects the current color; in the real app
       a project without a pick gets one by D-29 */
    for (const input of psSwatches.querySelectorAll('input')) {
      input.checked = input.value === p.color
    }
    for (const input of psSchemes.querySelectorAll('input')) {
      input.checked = input.value === (p.scheme ?? 'A')
    }
    psBaseFile.value = p.baseFile
    psExtraLocal.checked = !!p.extraLocal
    psExtraLocalPos.value = p.extraLocalPos ?? 'end'
    psExtraProject.checked = !!p.extraProject
    psExtraProjectPos.value = p.extraProjectPos ?? 'end'
    psPath.textContent = p.path
    psPath.title = p.path
    psAddedVia.textContent = p.addedVia
    psLastSync.textContent = p.syncedAt
    renderBackups(p)
    updateDerivedNames()
    updateSchemeUI()
    syncExtraRowUI()
    setFieldError(psName, psNameError, '')
    setFieldError(psLabel, psLabelError, '')
    selectPsTab(psTabGeneral, false)
    archiveInput.value = ''
    archiveConfirmBtn.disabled = true
    psDialog.showModal()
    psName.focus()
  }

  /* Saving: validate on the Save button click (preventing it keeps the
     dialog open with inline errors); on close apply to the column header */
  psForm.querySelector('button[value="save"]').addEventListener('click', (e) => {
    if (!validatePsFields()) e.preventDefault()
  })

  /* Enter in a text field saves — the implicit submitter would be Cancel */
  psForm.addEventListener('keydown', (e) => {
    const t = e.target
    if (e.key !== 'Enter' || t.tagName !== 'INPUT' || t.type === 'radio' || t.type === 'checkbox') {
      return
    }
    e.preventDefault()
    if (validatePsFields()) psDialog.close('save')
  })

  psDialog.addEventListener('close', () => {
    if (psDialog.returnValue !== 'save') return
    const p = PROJECTS.get(psCurrentName)
    if (!p) return
    const name = psName.value.trim()
    p.name = name
    p.label = psLabel.value.trim()
    p.description = psDescription.value
    p.color = psColorInput()?.value ?? p.color
    p.scheme = psSchemeInput()?.value ?? p.scheme
    p.baseFile = psBaseFile.value
    p.extraLocal = psExtraLocal.checked
    p.extraLocalPos = psExtraLocalPos.value
    p.extraProject = psExtraProject.checked
    p.extraProjectPos = psExtraProjectPos.value
    if (name !== psCurrentName) {
      PROJECTS.delete(psCurrentName)
      PROJECTS.set(name, p)
      psCurrentName = name
    }
    /* Live header update: color dot, name and the pencil tooltip; the th
       title (state + sync time) stays as-is */
    const th = p.editBtn?.closest('th')
    if (th) {
      th.querySelector('.proj-color')?.style.setProperty('--c', p.color)
      th.querySelector('.proj-name').textContent = name
      p.editBtn.title = `Edit project — ${name}`
      p.editBtn.setAttribute('aria-label', `Edit project — ${name}`)
    }
  })

  /* Tabs: click + roving arrow-key focus (WAI-ARIA tabs) */
  for (const tab of psTabs) {
    tab.addEventListener('click', () => selectPsTab(tab, false))
  }

  psTablist.addEventListener('keydown', (e) => {
    const current = psTabs.indexOf(document.activeElement)
    if (current === -1) return
    let next = null
    if (e.key === 'ArrowRight') next = (current + 1) % psTabs.length
    else if (e.key === 'ArrowLeft') next = (current + psTabs.length - 1) % psTabs.length
    else if (e.key === 'Home') next = 0
    else if (e.key === 'End') next = psTabs.length - 1
    if (next === null) return
    e.preventDefault()
    selectPsTab(psTabs[next], true)
  })

  /* Live field feedback */
  psName.addEventListener('input', () => setFieldError(psName, psNameError, ''))
  psLabel.addEventListener('input', () => {
    setFieldError(psLabel, psLabelError, '')
    psLabelPreview.textContent = psLabel.value.trim() || '—'
  })
  psDescription.addEventListener('input', () => {
    psDescCount.textContent = `${psDescription.value.length} / 1000`
  })
  psBaseFile.addEventListener('change', updateDerivedNames)
  for (const input of psSchemes.querySelectorAll('input')) {
    input.addEventListener('change', updateSchemeUI)
  }
  psExtraLocal.addEventListener('change', syncExtraRowUI)
  psExtraProject.addEventListener('change', syncExtraRowUI)

  /* Copy helpers (D-19 marker pair for scheme B, D-21 snippet for scheme C) */
  function wireCopyButton(btn, getText) {
    const original = btn.textContent
    btn.addEventListener('click', async () => {
      if (btn.dataset.busy) return
      btn.dataset.busy = 'true'
      try {
        await navigator.clipboard.writeText(getText())
        btn.textContent = 'Copied'
      } catch {
        btn.textContent = 'Copy failed'
      }
      setTimeout(() => {
        btn.textContent = original
        delete btn.dataset.busy
      }, 1500)
    })
  }

  wireCopyButton(document.getElementById('ps-copy-markers'), () =>
    ['<!-- EUNOMIASYNC:BEGIN -->', '<!-- EUNOMIASYNC:END -->'].join('\n'),
  )
  wireCopyButton(document.getElementById('ps-copy-snippet'), () => psSnippet.textContent)

  /* Danger zone: archive with type-to-confirm (PRJ-013…018). The demo is
     non-destructive: confirming just closes both dialogs. */
  psArchiveBtn.addEventListener('click', () => {
    const p = PROJECTS.get(psCurrentName)
    if (!p) return
    archiveText.textContent =
      `"${p.name}" will be archived. The project folder is not touched, assignments are kept, ` +
      'and the project stops taking part in syncs' +
      (p.addedVia === 'Found by scan' ? '; its path goes to the exclusions list' : '') +
      '. You can restore it later from the "Archived projects" screen.'
    archiveInput.value = ''
    archiveConfirmBtn.disabled = true
    archiveDialog.showModal()
    archiveInput.focus()
  })

  archiveInput.addEventListener('input', () => {
    archiveConfirmBtn.disabled = archiveInput.value.trim() !== psCurrentName
  })

  /* Enter in the confirm field archives (the implicit submitter would be
     the Cancel button) */
  archiveInput.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter') return
    e.preventDefault()
    if (!archiveConfirmBtn.disabled) archiveDialog.close('archive')
  })

  archiveDialog.addEventListener('close', () => {
    if (archiveDialog.returnValue === 'archive' && psDialog.open) {
      psDialog.close('archive')
    }
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

  /* ---------- Scan modal (PRJ-002, NFR-015) --------------------------------- */
  /* Toolbar «+ Add project»: the real app opens a folder picker first
     (PRJ-001); the prototype pretends ~/projects was picked and starts
     the scan right away. Phase 1 walks a pre-scripted demo list at
     120 ms per tick (under the 10 progress updates/s of SYS-003); phase
     2 shows the three result groups — New / Already added / Skipped
     (PRJ-002). Depth and exclusions are per D-04/D-05; a .git file and
     access errors follow D-06/PRJ-007. Closing the window (Cancel, Esc)
     stops the timer — closing means cancel — and re-opening always
     starts fresh. "Add selected" is a demo stub: it closes the dialog
     and never touches the matrix. */

  const scanDialog = document.getElementById('scan-dialog')
  const scanProgress = document.getElementById('scan-progress')
  const scanResults = document.getElementById('scan-results')
  const scanFoldersEl = document.getElementById('scan-count-folders')
  const scanReposEl = document.getElementById('scan-count-repos')
  const scanPathEl = document.getElementById('scan-current-path')
  const scanNewList = document.getElementById('scan-new-list')
  const scanKnownList = document.getElementById('scan-known-list')
  const scanSkippedList = document.getElementById('scan-skipped-list')
  const scanNewCount = document.getElementById('scan-new-count')
  const scanKnownCount = document.getElementById('scan-known-count')
  const scanSkippedCount = document.getElementById('scan-skipped-count')
  const scanAgainBtn = document.getElementById('scan-again-btn')
  const scanAddBtn = document.getElementById('scan-add-btn')
  const scanCancelBtn = document.getElementById('scan-cancel-btn')
  const addProjectBtn = document.getElementById('add-project-btn')
  const scanConfigEl = document.getElementById('scan-config')

  const SCAN_ROOT = '~/projects'
  const SCAN_TICK_MS = 120 // ≤10 progress updates per second (SYS-003)

  /* Repos the scan discovers that are not in the demo matrix yet */
  const SCAN_NEW = [
    { name: 'notes-api', path: 'notes/notes-api' },
    { name: 'landing-new', path: 'yepbro/landing-new' },
    { name: 'scripts-kit', path: 'scripts/scripts-kit' },
  ]

  /* Repos already registered as demo projects (no checkboxes) */
  const SCAN_KNOWN = [
    { name: 'EunomiaSync', path: 'business/EunomiaSync' },
    { name: 'yepbro/site', path: 'yepbro/site' },
    { name: 'quotameter', path: 'business/quotameter' },
    { name: 'task-id-mcp', path: 'utils/task-id-mcp' },
    { name: 'utils/deja-vu', path: 'utils/deja-vu' },
    { name: 'docktree', path: 'tools/docktree' },
    { name: 'moli', path: 'apps/moli' },
    { name: 'sandbox-cf', path: 'labs/sandbox-cf' },
  ]

  /* Skipped folders, one row per reason from the spec (D-05, D-06,
     PRJ-007); tone tints the badge, the reason text carries the meaning */
  const SCAN_SKIPPED = [
    {
      path: 'business/quotameter/node_modules',
      reason: 'excluded',
      hint: 'default excluded folder name (D-05)',
      tone: 'warn',
    },
    {
      path: 'libs/web-grid',
      reason: 'submodule or worktree',
      hint: '.git is a file — add it by hand if needed (D-06)',
      tone: 'warn',
    },
    {
      path: 'private/locked-vault',
      reason: 'no access',
      hint: 'os error 13 — permission denied; fix the rights and rescan',
      tone: 'bad',
    },
    {
      path: 'scratch/old-playground',
      reason: 'in exclusions',
      hint: 'the path was added to the exclusions list by hand',
      tone: '',
    },
    {
      path: 'attic/legacy-api',
      reason: 'archived',
      hint: 'already an archived project — restore it from the archived list',
      tone: '',
    },
  ]

  /* Pre-scripted depth-first walk under the simulated root (~40 folders,
     ~5 s). A plain path is just a folder checked; found: 'known' | 'new'
     | 'archived' is a repository discovery (bumps "Repositories found");
     skip is a folder passed over with a reason. Every repo of the three
     result groups is on this walk. */
  const SCAN_WALK = [
    { path: 'business' },
    { path: 'business/EunomiaSync', found: 'known' },
    { path: 'business/EunomiaSync/docs' },
    { path: 'business/quotameter', found: 'known' },
    { path: 'business/quotameter/node_modules', skip: true },
    { path: 'business/quotameter/src' },
    { path: 'business/kwork-ai' },
    { path: 'business/tasks-board' },
    { path: 'yepbro' },
    { path: 'yepbro/site', found: 'known' },
    { path: 'yepbro/site/src' },
    { path: 'yepbro/landing-new', found: 'new' },
    { path: 'tools' },
    { path: 'tools/docktree', found: 'known' },
    { path: 'tools/docktree/docs' },
    { path: 'apps' },
    { path: 'apps/moli', found: 'known' },
    { path: 'apps/gallery-old' },
    { path: 'utils' },
    { path: 'utils/task-id-mcp', found: 'known' },
    { path: 'utils/task-id-mcp/src' },
    { path: 'utils/deja-vu', found: 'known' },
    { path: 'labs' },
    { path: 'labs/sandbox-cf', found: 'known' },
    { path: 'labs/sandbox-cf/tests' },
    { path: 'notes' },
    { path: 'notes/notes-api', found: 'new' },
    { path: 'scripts' },
    { path: 'scripts/scripts-kit', found: 'new' },
    { path: 'libs' },
    { path: 'libs/web-grid', skip: true },
    { path: 'private' },
    { path: 'private/locked-vault', skip: true },
    { path: 'scratch' },
    { path: 'scratch/old-playground', skip: true },
    { path: 'attic' },
    { path: 'attic/legacy-api', found: 'archived' },
    { path: 'tmp-sandbox' },
    { path: 'website-archive' },
    { path: 'playground' },
  ]

  /* Live scan state; the timer is the only thing that must never survive
     a close (NFR-015: closing the scan window means cancel) */
  let scanTimer = null
  let scanIdx = 0
  let scanReposFound = 0

  function stopScanTimer() {
    if (scanTimer === null) return
    clearInterval(scanTimer)
    scanTimer = null
  }

  function scanTick() {
    const step = SCAN_WALK[scanIdx]
    scanIdx += 1
    scanFoldersEl.textContent = String(scanIdx)
    scanPathEl.textContent = `${SCAN_ROOT}/${step.path}`
    if (step.found) {
      scanReposFound += 1
      scanReposEl.textContent = String(scanReposFound)
    }
    if (scanIdx >= SCAN_WALK.length) {
      stopScanTimer()
      renderScanResults()
    }
  }

  /* Fresh phase 1: called on open and by "Scan again" — the stopScanTimer
     guard keeps there from ever being two running timers. The static
     config line reads the demo SETTINGS so the depth and exclusions
     edited in the Settings modal (D-04, D-05) show up on the next scan. */
  function startScan() {
    stopScanTimer()
    scanConfigEl.textContent = `Depth ${SETTINGS.scan.maxDepth} · Excluded: ${SETTINGS.scan.excludeNames.join(', ')}`
    scanIdx = 0
    scanReposFound = 0
    scanFoldersEl.textContent = '0'
    scanReposEl.textContent = '0'
    scanPathEl.textContent = SCAN_ROOT
    scanNewList.replaceChildren()
    scanKnownList.replaceChildren()
    scanSkippedList.replaceChildren()
    scanProgress.hidden = false
    scanResults.hidden = true
    scanAgainBtn.hidden = true
    scanAddBtn.hidden = true
    scanAddBtn.disabled = true
    scanTimer = setInterval(scanTick, SCAN_TICK_MS)
  }

  /* Phase 2 (PRJ-002): strings go through escapeHtml, counters through
     textContent only */
  function renderScanResults() {
    const rowPath = (p) => escapeHtml(`${SCAN_ROOT}/${p}`)
    scanNewList.innerHTML = SCAN_NEW.map(
      (r) =>
        '<label class="scan-row">' +
        '<input type="checkbox" class="scan-check" checked />' +
        `<span class="scan-row-name">${escapeHtml(r.name)}</span>` +
        `<code class="scan-row-path">${rowPath(r.path)}</code>` +
        '</label>',
    ).join('')
    scanKnownList.innerHTML = SCAN_KNOWN.map(
      (r) =>
        '<div class="scan-row scan-row-known">' +
        `<span class="scan-row-name">${escapeHtml(r.name)}</span>` +
        `<code class="scan-row-path">${rowPath(r.path)}</code>` +
        '</div>',
    ).join('')
    scanSkippedList.innerHTML = SCAN_SKIPPED.map(
      (r) =>
        '<div class="scan-row scan-row-skip">' +
        `<span class="chip scan-reason${r.tone ? ` scan-reason--${r.tone}` : ''}">${escapeHtml(r.reason)}</span>` +
        `<code class="scan-row-path">${rowPath(r.path)}</code>` +
        `<span class="scan-reason-hint">${escapeHtml(r.hint)}</span>` +
        '</div>',
    ).join('')
    scanNewCount.textContent = String(SCAN_NEW.length)
    scanKnownCount.textContent = String(SCAN_KNOWN.length)
    scanSkippedCount.textContent = String(SCAN_SKIPPED.length)
    scanProgress.hidden = true
    scanResults.hidden = false
    scanAgainBtn.hidden = false
    scanAddBtn.hidden = false
    syncScanAddBtn()
  }

  /* "Add selected" is disabled while nothing is checked */
  function syncScanAddBtn() {
    scanAddBtn.disabled = scanResults.querySelectorAll('.scan-check:checked').length === 0
  }

  /* Toolbar entry: simulated picked root, straight into phase 1 */
  addProjectBtn.addEventListener('click', () => {
    startScan()
    scanDialog.showModal()
  })

  scanResults.addEventListener('change', (e) => {
    if (e.target.closest('.scan-check')) syncScanAddBtn()
  })

  /* Cancel / Esc / "Add selected" all funnel through close: stopping the
     timer on the close event makes closing the window equal to cancel
     (NFR-015; PRJ-002 AC: canceling adds no projects) */
  scanDialog.addEventListener('close', stopScanTimer)
  scanCancelBtn.addEventListener('click', () => scanDialog.close('cancel'))
  scanAddBtn.addEventListener('click', () => scanDialog.close('add'))
  scanAgainBtn.addEventListener('click', startScan)

  /* ---------- Sync summary modal (MAIN-024…031, OUT-009) ------------------- */
  /* "Run sync" starts a simulated run: the button is disabled (MAIN-030;
     SYS-004 — one run at a time, guarded by the syncRunning flag too),
     an adjacent live region shows "k of N" (SYS-003: ≤10 updates/s),
     and every tick applies one project's pending assignments — the same
     reset the old instant-apply handler did (dataset.initial /
     initialVariant + tint), scoped to that column. Outcomes mirror the
     demo column states per §6.2 sync_run_projects: written / unchanged
     (D-24) / skipped — the run continues (D-09, SYS-009) / failed with a
     kept backup (§10). At the end the dirty badge clears and the summary
     opens in "result" mode; written/unchanged projects get fresh
     per-project times (DB-024) while a partial run shows "Last attempt:
     partial" in the sync bar instead of a new overall time (§9.4 phase 4,
     LOG-007). The counter chip opens the same dialog in "pending" mode
     (MAIN-026). */

  const ssDialog = document.getElementById('sync-summary-dialog')
  const ssTitle = document.getElementById('ss-title')
  const ssResult = document.getElementById('ss-result')
  const ssPending = document.getElementById('ss-pending')
  const ssRunBadge = document.getElementById('ss-run-badge')
  const ssRunNote = document.getElementById('ss-run-note')
  const ssNWritten = document.getElementById('ss-n-written')
  const ssNUchanged = document.getElementById('ss-n-unchanged')
  const ssNSkipped = document.getElementById('ss-n-skipped')
  const ssNFailed = document.getElementById('ss-n-failed')
  const ssRows = document.getElementById('ss-rows')
  const ssPendingList = document.getElementById('ss-pending-list')
  const ssPendingRows = document.getElementById('ss-pending-rows')
  const ssPendingEmpty = document.getElementById('ss-pending-empty')
  const ssRunBtn = document.getElementById('ss-run-btn')
  const syncProgress = document.getElementById('sync-progress')
  const syncProgressText = document.getElementById('sync-progress-text')
  const syncLastEl = document.querySelector('.syncbar .sync-last')
  const syncCounterBtn = document.querySelector('.syncbar .sync-counter')

  const SYNC_TICK_MS = 200 // ≤10 progress updates per second (SYS-003); 8 ticks ≈ 1.6 s

  /* Shared wording for the four demo projects that get written */
  const SS_WRITTEN_REASON = 'Pending assignments were applied — the target file was written.'

  /* Demo outcome per project, keyed by name (§6.2 sync_run_projects
     statuses + §10 error codes; D-09 — skips do not stop the run) */
  const SYNC_OUTCOMES = {
    EunomiaSync: { status: 'written', reason: SS_WRITTEN_REASON },
    'yepbro/site': {
      status: 'unchanged',
      reason: 'The built content matches the file, so it was not overwritten (D-24).',
    },
    quotameter: {
      status: 'skipped',
      code: 'E_MARKERS_INVALID',
      reason:
        'Scheme B markers are missing or invalid in the target file — nothing was written (SYS-009).',
      hint: 'Paste the marker block with “Copy marker block” in the project settings.',
    },
    'task-id-mcp': { status: 'written', reason: SS_WRITTEN_REASON },
    'utils/deja-vu': {
      status: 'skipped',
      code: 'E_PROJECT_UNAVAILABLE',
      reason:
        'The project folder is unavailable (moved or the drive is disconnected) — the project was skipped (D-09).',
      hint: 'Use “Locate…” to set the new path, or archive the project.',
    },
    docktree: { status: 'written', reason: SS_WRITTEN_REASON },
    moli: { status: 'written', reason: SS_WRITTEN_REASON },
    'sandbox-cf': {
      status: 'failed',
      code: 'E_FILE_LOCKED',
      reason:
        'The target file is locked by another process — 3 write retries failed; the backup is kept (§10).',
      hint: 'Close the program that locks the file and run sync again.',
    },
  }

  /* Run state: the timer is the only thing that must never survive the
     run or the page (no dialog is open mid-run, so unlike the scan there
     is no close event — the final tick stops the timer itself) */
  let syncTimer = null
  let syncRunning = false

  function stopSyncTimer() {
    if (syncTimer !== null) clearInterval(syncTimer)
    syncTimer = null
  }

  /* Applies the pending assignments of one project column: changed cells
     become the new initial state and lose the tint (the per-column
     version of the old instant-apply loop) */
  function applyColumnAssignments(colIdx) {
    for (const input of document.querySelectorAll('.matrix tbody .assign')) {
      const td = input.closest('td')
      if (td.cellIndex - 2 !== colIdx) continue
      const select = td.querySelector('.variant-select')
      if (select) select.dataset.initialVariant = select.value
      input.dataset.initial = String(input.checked)
      updateCellState(input)
    }
  }

  /* Demo clock, like the history panel: rendered in the fixed UTC+3 the
     demo uses, both in the sync bar format and the short column format */
  function syncStampNow() {
    const d = new Date(Date.now() + 180 * 60000)
    const hh = String(d.getUTCHours()).padStart(2, '0')
    const mm = String(d.getUTCMinutes()).padStart(2, '0')
    const day = d.getUTCDate()
    const mon = MONTHS_SHORT[d.getUTCMonth()]
    return {
      full: `${day} ${mon} ${d.getUTCFullYear()}, ${hh}:${mm} (UTC+3)`,
      short: `${day} ${mon}, ${hh}:${mm}`,
    }
  }

  /* Target file of a project: <~path>/<base file>, scheme C writes
     <STEM>.project.md instead (§9.4 preflight); the ~ mirrors the demo
     paths of the scan modal */
  function targetPathOf(name) {
    const p = PROJECTS.get(name)
    if (!p) return 'AGENTS.md'
    const file = p.scheme === 'C' ? `${stemOf(p.baseFile)}.project.md` : p.baseFile
    return `${p.path.replace(/^\/home\/brooks/, '~')}/${file}`
  }

  /* Final run status (§9.4 phase 4): success if every active project was
     written or left unchanged, partial otherwise */
  function runOutcomeOf(results) {
    return results.some((r) => r.status === 'skipped' || r.status === 'failed')
      ? 'partial'
      : 'success'
  }

  /* The sync bar's overall line (MAIN-027, §9.4 phase 4, LOG-007): only a
     successful run moves the "Last successful sync" time; a partial one
     shows the "Last attempt: partial" note instead */
  function renderSyncLast(outcome, stamp) {
    syncLastEl.innerHTML =
      outcome === 'success'
        ? `Last successful sync: <strong>${escapeHtml(stamp.full)}</strong>`
        : 'Last attempt: <strong>partial</strong>'
  }

  function startSyncRun() {
    if (syncRunning) return // MAIN-030/SYS-004: a second start is not possible
    const results = projectHeads.map((th, idx) => {
      const name = th.querySelector('.proj-name')?.textContent.trim() ?? ''
      return { idx, name, ...SYNC_OUTCOMES[name] }
    })
    syncRunning = true
    runSyncBtn.disabled = true
    syncProgress.hidden = false
    syncProgressText.textContent = `0 of ${results.length}`
    let done = 0
    syncTimer = setInterval(() => {
      if (results[done].status === 'written') applyColumnAssignments(results[done].idx)
      done += 1
      syncProgressText.textContent = `${done} of ${results.length}`
      if (done >= results.length) {
        stopSyncTimer()
        finishSyncRun(results)
      }
    }, SYNC_TICK_MS)
  }

  function finishSyncRun(results) {
    syncRunning = false
    const stamp = syncStampNow()
    /* Per-project times (§9.4 phase 4, DB-024): written/unchanged
       projects get a fresh last_synced_at — the column tooltips and the
       project settings modal follow; skipped/failed keep theirs */
    for (const r of results) {
      if (r.status !== 'written' && r.status !== 'unchanged') continue
      const p = PROJECTS.get(r.name)
      if (p) p.syncedAt = stamp.full
      const st = projectColsState[r.idx]
      st.time = stamp.short
      /* A clean column that showed a plain time (no warning icon) shows
         the new one; warning subs keep their markup */
      if (st.cleanHtml && !st.cleanHtml.includes('<')) st.cleanHtml = stamp.short
      projectHeads[r.idx].title = `${r.name} — last successful sync: ${stamp.full}`
    }
    renderSyncLast(runOutcomeOf(results), stamp)
    refreshDirtyState() // applied cells are the new initial state: badge -> 0
    runSyncBtn.disabled = false
    syncProgress.hidden = true
    openSyncSummary({ mode: 'result', results })
  }

  /* One summary row: status badge, then for skips/failures the error
     code chip and the "what to do" hint (UI-011). Strings go through
     escapeHtml even though they come from the demo data. */
  function renderSyncResults(results) {
    ssRows.innerHTML = results
      .map((r) => {
        const code = r.code ? `<span class="chip ss-code">${escapeHtml(r.code)}</span>` : ''
        const hint = r.hint ? `<span class="ss-hint">${escapeHtml(r.hint)}</span>` : ''
        const extra = code || hint ? `<span class="ss-code-row">${code}${hint}</span>` : ''
        return (
          '<tr>' +
          `<td class="ss-proj">${escapeHtml(r.name)}</td>` +
          `<td class="ss-file"><code>${escapeHtml(targetPathOf(r.name))}</code></td>` +
          `<td class="ss-st-cell"><span class="chip ss-st ss-st--${escapeHtml(r.status)}">${escapeHtml(r.status)}</span></td>` +
          `<td class="ss-reason"><span class="ss-reason-text">${escapeHtml(r.reason)}</span>${extra}</td>` +
          '</tr>'
        )
      })
      .join('')
  }

  /* Pending mode (MAIN-026): the currently dirty projects and their
     target files; the "Run sync" convenience acts only when the list is
     not empty */
  function renderPendingList() {
    const names = projectColsState
      .map((c, i) => (c.dirty ? i : -1))
      .filter((i) => i >= 0)
      .map((i) => projectHeads[i].querySelector('.proj-name')?.textContent.trim() ?? '')
    ssPendingRows.innerHTML = names
      .map(
        (name) =>
          '<tr>' +
          `<td class="ss-proj">${escapeHtml(name)}</td>` +
          `<td class="ss-file"><code>${escapeHtml(targetPathOf(name))}</code></td>` +
          '</tr>',
      )
      .join('')
    ssPendingList.hidden = names.length === 0
    ssPendingEmpty.hidden = names.length !== 0
    ssRunBtn.disabled = names.length === 0
  }

  /* Both modes share the dialog chrome; the title swaps accordingly */
  function openSyncSummary({ mode, results = [] }) {
    const isResult = mode === 'result'
    ssResult.hidden = !isResult
    ssPending.hidden = isResult
    ssRunBtn.hidden = isResult
    if (isResult) {
      ssTitle.textContent = 'Sync summary'
      const n = (s) => results.filter((r) => r.status === s).length
      const skipped = n('skipped')
      const failed = n('failed')
      const outcome = runOutcomeOf(results)
      ssRunBadge.textContent = outcome
      ssRunBadge.className = `chip ss-run-badge ss-run-badge--${outcome}`
      ssRunNote.textContent =
        outcome === 'partial'
          ? 'Last attempt: partial — skips and failures do not stop the run (D-09, §9.4).'
          : 'Every active project was written or left unchanged.'
      ssNWritten.textContent = String(n('written'))
      ssNUchanged.textContent = String(n('unchanged'))
      ssNSkipped.textContent = String(skipped)
      ssNFailed.textContent = String(failed)
      renderSyncResults(results)
    } else {
      ssTitle.textContent = 'Unsynced changes'
      renderPendingList()
    }
    if (!ssDialog.open) ssDialog.showModal()
  }

  /* Entry point 1: the counter chip lists the projects with unsynced
     changes (MAIN-026); mid-run there is nothing stable to list */
  syncCounterBtn.addEventListener('click', () => {
    if (syncRunning) return
    openSyncSummary({ mode: 'pending' })
  })

  /* Entry point 2 (MAIN-026 convenience): close and start the same run */
  ssRunBtn.addEventListener('click', () => {
    ssDialog.close('run')
    startSyncRun()
  })

  /* No dialog is open during the run itself; make sure no orphan timer
     can re-fire when the page goes away (scan-modal cleanup idiom) */
  window.addEventListener('pagehide', stopSyncTimer)

  /* ---------- Settings modal (DEF-001…004, §6.5) --------------------------- */
  /* Opened by the toolbar ghost button and Ctrl/Cmd + , (§11.3). Five
     vertical sections (defaults, backups, scanning, history/logs,
     updates) instead of tabs. Every control writes into the demo
     SETTINGS object right away — there is no Save button, settings
     apply without a restart (DEF-001 AC); changing default.* never
     touches the existing PROJECTS (DEF-005). Numbers outside their
     §6.5 range show an inline error and keep the previous value until
     valid (error, not clamp — the same behavior for every field).
     D-18 (export/import of settings in JSON) has no UI in this
     prototype; sync.skip_identical and ui.language live in SETTINGS
     without a control. "About…" / "Check for updates" open the generic
     placeholder dialog for now (UPD-001, UPD-005); the footer row also
     holds the D-10 entry buttons — "Archived projects…" opens the
     archived list directly, "Excluded paths…" opens the exclusions
     list. */

  /* §6.5 defaults (the real app keeps them in the settings table as
     JSON — D-18); key names mirror the table, camelCased */
  const SETTINGS = {
    default: {
      scheme: 'A',
      baseName: 'AGENTS.md',
      extraLocal: false,
      extraLocalPos: 'end',
      extraProject: false,
      extraProjectPos: 'end',
    },
    backup: { retention: 10 },
    sync: { skipIdentical: true },
    scan: {
      maxDepth: 6,
      excludeNames: ['node_modules', 'vendor', '.git', 'dist', 'build', 'target', '.venv'],
    },
    history: { maxRevisions: 200 },
    log: { maxEntries: 10000, maxDays: 180 },
    ui: { language: 'en' },
    update: { checkOnStart: true },
  }

  const settingsDialog = document.getElementById('settings-dialog')
  const settingsBtn = document.getElementById('settings-btn')
  const setForm = document.getElementById('set-form')
  const setSchemes = document.getElementById('set-schemes')
  const setBaseFile = document.getElementById('set-base-file')
  const setExtraFiles = document.getElementById('set-extra-files')
  const setExtraLocal = document.getElementById('set-extra-local')
  const setExtraLocalPos = document.getElementById('set-extra-local-pos')
  const setExtraProject = document.getElementById('set-extra-project')
  const setExtraProjectPos = document.getElementById('set-extra-project-pos')
  const setExtraLocalName = document.getElementById('set-extra-local-name')
  const setExtraProjectName = document.getElementById('set-extra-project-name')
  const setBackupRetention = document.getElementById('set-backup-retention')
  const setBackupRetentionError = document.getElementById('set-backup-retention-error')
  const setScanDepth = document.getElementById('set-scan-depth')
  const setScanDepthError = document.getElementById('set-scan-depth-error')
  const setHistRevisions = document.getElementById('set-hist-revisions')
  const setHistRevisionsError = document.getElementById('set-hist-revisions-error')
  const setLogEntries = document.getElementById('set-log-entries')
  const setLogEntriesError = document.getElementById('set-log-entries-error')
  const setLogDays = document.getElementById('set-log-days')
  const setLogDaysError = document.getElementById('set-log-days-error')
  const setExclList = document.getElementById('set-excl-list')
  const setExclInput = document.getElementById('set-excl-input')
  const setExclAddBtn = document.getElementById('set-excl-add-btn')
  const setExclError = document.getElementById('set-excl-error')
  const setUpdateCheck = document.getElementById('set-update-check')

  /* Number fields with §6.5 ranges: invalid input -> inline error, the
     previous value stays in SETTINGS until the field is valid again */
  const SET_NUMBER_FIELDS = [
    {
      input: setBackupRetention,
      error: setBackupRetentionError,
      label: 'Backup limit',
      min: 1,
      max: 200,
      get: () => SETTINGS.backup.retention,
      set: (n) => (SETTINGS.backup.retention = n),
    },
    {
      input: setScanDepth,
      error: setScanDepthError,
      label: 'Scan depth',
      min: 1,
      max: 20,
      get: () => SETTINGS.scan.maxDepth,
      set: (n) => (SETTINGS.scan.maxDepth = n),
    },
    {
      input: setHistRevisions,
      error: setHistRevisionsError,
      label: 'Versions per entity',
      min: 10,
      max: 1000,
      get: () => SETTINGS.history.maxRevisions,
      set: (n) => (SETTINGS.history.maxRevisions = n),
    },
    {
      input: setLogEntries,
      error: setLogEntriesError,
      label: 'Log entries',
      min: 1000,
      max: 100000,
      get: () => SETTINGS.log.maxEntries,
      set: (n) => (SETTINGS.log.maxEntries = n),
    },
    {
      input: setLogDays,
      error: setLogDaysError,
      label: 'Log days',
      min: 7,
      max: 3650,
      get: () => SETTINGS.log.maxDays,
      set: (n) => (SETTINGS.log.maxDays = n),
    },
  ]

  /* Scheme A extras follow the base file name (D-20) and hide for B/C,
     like in the project settings modal */
  function updateSetDerivedNames() {
    const stem = stemOf(setBaseFile.value)
    setExtraLocalName.textContent = `${stem}.local.md`
    setExtraProjectName.textContent = `${stem}.project.md`
  }

  function updateSetSchemeUI() {
    const scheme = setSchemes.querySelector('input:checked')?.value ?? 'A'
    setExtraFiles.hidden = scheme !== 'A'
  }

  function syncSetExtraRows() {
    setExtraLocalPos.disabled = !setExtraLocal.checked
    setExtraProjectPos.disabled = !setExtraProject.checked
  }

  /* Excluded names (D-05): one chip row per name with an accessible
     remove button; names are user-typed, so they go through escapeHtml */
  function renderExclList() {
    setExclList.innerHTML = SETTINGS.scan.excludeNames
      .map(
        (name, i) =>
          '<div class="set-excl-row">' +
          `<span class="chip">${escapeHtml(name)}</span>` +
          `<button type="button" class="cell-icon cell-icon--delete set-excl-remove" data-idx="${i}"` +
          ` title="Remove ${escapeHtml(name)}"` +
          ` aria-label="Remove ${escapeHtml(name)} from excluded folder names">` +
          ICONS.delete +
          '</button>' +
          '</div>',
      )
      .join('')
  }

  /* Add is validated: trimmed, non-empty (no whitespace-only names), no
     duplicates; the error clears as soon as the input changes */
  function addExclusion() {
    const name = setExclInput.value.trim()
    if (name === '') {
      setFieldError(
        setExclInput,
        setExclError,
        'Enter a folder name — empty or whitespace-only names are not allowed.',
      )
      return
    }
    if (SETTINGS.scan.excludeNames.includes(name)) {
      setFieldError(setExclInput, setExclError, `"${name}" is already in the excluded list.`)
      return
    }
    SETTINGS.scan.excludeNames.push(name)
    setExclInput.value = ''
    setFieldError(setExclInput, setExclError, '')
    renderExclList()
    setExclInput.focus()
  }

  function wireSetNumber({ input, error, label, min, max, set }) {
    input.addEventListener('input', () => {
      const raw = input.value.trim()
      const n = Number(raw)
      if (raw === '' || !Number.isInteger(n) || n < min || n > max) {
        setFieldError(input, error, `${label}: an integer from ${min} to ${max} (§6.5).`)
        return
      }
      setFieldError(input, error, '')
      set(n)
    })
  }

  /* Re-sync the controls from SETTINGS on every open: reopening shows the
     canonical values and drops any invalid text left in a number field */
  function loadSettingsUI() {
    for (const input of setSchemes.querySelectorAll('input')) {
      input.checked = input.value === SETTINGS.default.scheme
    }
    setBaseFile.value = SETTINGS.default.baseName
    setExtraLocal.checked = SETTINGS.default.extraLocal
    setExtraLocalPos.value = SETTINGS.default.extraLocalPos
    setExtraProject.checked = SETTINGS.default.extraProject
    setExtraProjectPos.value = SETTINGS.default.extraProjectPos
    for (const f of SET_NUMBER_FIELDS) {
      f.input.value = String(f.get())
      setFieldError(f.input, f.error, '')
    }
    setUpdateCheck.checked = SETTINGS.update.checkOnStart
    setExclInput.value = ''
    setFieldError(setExclInput, setExclError, '')
    updateSetDerivedNames()
    updateSetSchemeUI()
    syncSetExtraRows()
    renderExclList()
  }

  function openSettings() {
    loadSettingsUI()
    settingsDialog.showModal()
    setSchemes.querySelector('input:checked')?.focus()
  }

  /* Entry point 1: the toolbar ghost button */
  settingsBtn.addEventListener('click', openSettings)

  /* Entry point 2 (§11.3): Ctrl/Cmd + , . Ignored while the settings
     dialog is already open; if another dialog is open, the native
     <dialog> stacking puts settings on top. */
  document.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey || e.metaKey) || e.key !== ',') return
    e.preventDefault()
    if (!settingsDialog.open) openSettings()
  })

  /* Close is the only footer action; everything else applied on change */
  setForm.querySelector('button[value="close"]').addEventListener('click', () => {
    settingsDialog.close('close')
  })

  /* Enter in a text field must not close the dialog (the implicit
     submitter is Close); in the exclusion input it adds the name */
  setForm.addEventListener('keydown', (e) => {
    const t = e.target
    if (e.key !== 'Enter' || t.tagName !== 'INPUT' || t.type === 'radio' || t.type === 'checkbox') {
      return
    }
    e.preventDefault()
    if (t === setExclInput) addExclusion()
  })

  /* Defaults: apply on change, never touch existing projects (DEF-005) */
  for (const input of setSchemes.querySelectorAll('input')) {
    input.addEventListener('change', () => {
      SETTINGS.default.scheme = input.value
      updateSetSchemeUI()
    })
  }
  setBaseFile.addEventListener('change', () => {
    SETTINGS.default.baseName = setBaseFile.value
    updateSetDerivedNames()
  })
  setExtraLocal.addEventListener('change', () => {
    SETTINGS.default.extraLocal = setExtraLocal.checked
    syncSetExtraRows()
  })
  setExtraLocalPos.addEventListener('change', () => {
    SETTINGS.default.extraLocalPos = setExtraLocalPos.value
  })
  setExtraProject.addEventListener('change', () => {
    SETTINGS.default.extraProject = setExtraProject.checked
    syncSetExtraRows()
  })
  setExtraProjectPos.addEventListener('change', () => {
    SETTINGS.default.extraProjectPos = setExtraProjectPos.value
  })
  setUpdateCheck.addEventListener('change', () => {
    SETTINGS.update.checkOnStart = setUpdateCheck.checked
  })

  for (const f of SET_NUMBER_FIELDS) wireSetNumber(f)

  /* Exclusion list: remove buttons + add row (button click or Enter) */
  setExclList.addEventListener('click', (e) => {
    const btn = e.target.closest('.set-excl-remove')
    if (!btn) return
    const i = Number(btn.dataset.idx)
    if (!Number.isInteger(i) || i < 0 || i >= SETTINGS.scan.excludeNames.length) return
    SETTINGS.scan.excludeNames.splice(i, 1)
    renderExclList()
  })

  setExclAddBtn.addEventListener('click', addExclusion)
  setExclInput.addEventListener('input', () => setFieldError(setExclInput, setExclError, ''))

  /* ---------- Archived projects modal (D-10) ------------------------------- */
  /* The D-10 screen: every archived project with a Restore action.
     Archived projects keep their assignments, do not take part in syncs
     and their folders are not touched (DB-014); restoring (PRJ-018)
     brings the project back active and removes its path from the
     exclusions so a scan finds it again, and adding an archived folder
     again offers to restore it (DB-015). The demo state is an in-memory
     list: restore removes the row and never touches the matrix. */

  /* Demo archived projects; colors are fixed-palette (DB-040) values and
     addedVia matches the wording of the project settings modal */
  const ARCHIVED = [
    {
      name: 'old-crm',
      path: '~/projects/legacy/old-crm',
      color: '#8b5cf6', // Violet
      archivedAt: '3 Nov 2026',
      addedVia: 'Found by scan',
    },
    {
      name: 'legacy-parser',
      path: '~/projects/legacy/legacy-parser',
      color: '#f59e0b', // Amber
      archivedAt: '28 Oct 2026',
      addedVia: 'Manual',
    },
    {
      name: 'wiki-bot',
      path: '~/projects/apps/wiki-bot',
      color: '#06b6d4', // Cyan
      archivedAt: '12 Oct 2026',
      addedVia: 'Found by scan',
    },
  ]

  const arDialog = document.getElementById('archived-dialog')
  const arList = document.getElementById('ar-rows')
  const arEmpty = document.getElementById('ar-empty')
  const arCloseBtn = arDialog.querySelector('button[value="close"]')
  const arRestoreDialog = document.getElementById('restore-project-dialog')
  const arRestoreText = document.getElementById('ar-restore-text')

  /* One row per archived project, in the scan-results row idiom; every
     string goes through escapeHtml. The Restore button repeats the
     project name for screen readers (identical visible labels). */
  function renderArchivedRows() {
    arList.innerHTML = ARCHIVED.map(
      (p, i) =>
        '<li class="scan-row">' +
        `<span class="proj-color" style="--c: ${p.color}" aria-hidden="true"></span>` +
        `<span class="scan-row-name">${escapeHtml(p.name)}</span>` +
        `<code class="scan-row-path">${escapeHtml(p.path)}</code>` +
        `<span class="ar-meta">Archived ${escapeHtml(p.archivedAt)}</span>` +
        `<span class="ar-meta">${escapeHtml(p.addedVia)}</span>` +
        '<button type="button" class="btn btn-ghost ar-restore"' +
        ` data-idx="${i}" aria-label="Restore ${escapeHtml(p.name)}">Restore</button>` +
        '</li>',
    ).join('')
    const empty = ARCHIVED.length === 0
    arList.hidden = empty
    arEmpty.hidden = !empty
  }

  /* Re-render on every open: reopening reflects the current demo state */
  function openArchivedProjects() {
    renderArchivedRows()
    arDialog.showModal()
    arCloseBtn.focus()
  }

  /* Entry point: ghost link-button in the settings footer */
  document.getElementById('archived-open-btn').addEventListener('click', openArchivedProjects)

  arCloseBtn.addEventListener('click', () => {
    arDialog.close('close')
  })

  /* Restore (PRJ-018): confirm first in the dedicated small dialog — the
     shared #confirm-dialog is not reusable, its button is hard-labeled
     "Delete". Cancel/Esc leave the list untouched. */
  let pendingArRestore = null

  arList.addEventListener('click', (e) => {
    const btn = e.target.closest('.ar-restore')
    if (!btn) return
    const i = Number(btn.dataset.idx)
    if (!Number.isInteger(i) || i < 0 || i >= ARCHIVED.length) return
    const p = ARCHIVED[i]
    pendingArRestore = i
    arRestoreText.textContent =
      `"${p.name}" will be restored. The project comes back active with all its assignments, ` +
      'and its path is removed from the excluded paths so a scan can find it again.'
    arRestoreDialog.showModal()
  })

  arRestoreDialog.addEventListener('close', () => {
    const i = pendingArRestore
    pendingArRestore = null
    if (i === null || arRestoreDialog.returnValue !== 'restore') return
    ARCHIVED.splice(i, 1)
    renderArchivedRows()
    /* The confirmed row is gone from the DOM: keep focus inside the dialog */
    const nextFocus = arList.querySelector('.ar-restore') ?? arCloseBtn
    nextFocus.focus()
  })

  /* ---------- Excluded paths modal (D-10) ---------------------------------- */
  /* The second D-10 screen: every excluded path with a Remove action.
     Entries mirror the §6.2 project_exclusions DDL (path, reason IN
     ('removed_scanned','manual'), created_at): a scanned project that
     was archived leaves its path here (PRJ-017) and the scan shows such
     paths in its "Skipped" group (PRJ-002). Removing an entry (§6.4,
     exclusion_remove) deletes it, so the path is found by a scan again.
     The demo state is an in-memory list: remove splices the row and
     never touches the matrix. */

  /* Demo exclusions in DDL shape; the reason labels below carry the
     meaning, the badge tint only supports them */
  const EXCLUSIONS = [
    { path: '~/projects/archive/old-sandbox', reason: 'removed_scanned', addedAt: '5 Nov 2026' },
    { path: '~/projects/experiments/prototype-v0', reason: 'manual', addedAt: '1 Nov 2026' },
    { path: '~/vendor-legacy', reason: 'manual', addedAt: '20 Oct 2026' },
  ]

  /* Human wording per reason (§6.2 enum), in the wording style of the
     archived modal's "Found by scan" / "Manual" */
  const EXCLUSION_REASONS = {
    removed_scanned: 'Removed by archiving',
    manual: 'Added manually',
  }

  const exDialog = document.getElementById('exclusions-dialog')
  const exList = document.getElementById('ex-rows')
  const exEmpty = document.getElementById('ex-empty')
  const exCloseBtn = exDialog.querySelector('button[value="close"]')
  const exRemoveDialog = document.getElementById('remove-exclusion-dialog')
  const exRemoveText = document.getElementById('ex-remove-text')

  /* One row per excluded path in the archived row idiom (.scan-row +
     .ar-meta); every string goes through escapeHtml. The Remove button
     repeats the path for screen readers (identical visible labels). */
  function renderExclusionRows() {
    exList.innerHTML = EXCLUSIONS.map(
      (x, i) =>
        '<li class="scan-row">' +
        `<code class="scan-row-path">${escapeHtml(x.path)}</code>` +
        `<span class="chip ex-reason ex-reason--${escapeHtml(x.reason)}">${escapeHtml(EXCLUSION_REASONS[x.reason] ?? x.reason)}</span>` +
        `<span class="ar-meta">Added ${escapeHtml(x.addedAt)}</span>` +
        '<button type="button" class="btn btn-ghost ex-remove"' +
        ` data-idx="${i}" aria-label="Remove ${escapeHtml(x.path)} from excluded paths">Remove</button>` +
        '</li>',
    ).join('')
    const empty = EXCLUSIONS.length === 0
    exList.hidden = empty
    exEmpty.hidden = !empty
  }

  /* Re-render on every open: reopening reflects the current demo state */
  function openExcludedPaths() {
    renderExclusionRows()
    exDialog.showModal()
    exCloseBtn.focus()
  }

  /* Entry point: ghost link-button in the settings footer */
  document.getElementById('exclusions-open-btn').addEventListener('click', openExcludedPaths)

  exCloseBtn.addEventListener('click', () => {
    exDialog.close('close')
  })

  /* Remove (D-10, §6.4): confirm first in the dedicated small dialog —
     the shared #confirm-dialog is not reusable, its button is
     hard-labeled "Delete". Cancel/Esc leave the list untouched. */
  let pendingExRemove = null

  exList.addEventListener('click', (e) => {
    const btn = e.target.closest('.ex-remove')
    if (!btn) return
    const i = Number(btn.dataset.idx)
    if (!Number.isInteger(i) || i < 0 || i >= EXCLUSIONS.length) return
    const x = EXCLUSIONS[i]
    pendingExRemove = i
    exRemoveText.textContent =
      `"${x.path}" will be removed from the excluded paths. The path will be found by the scan ` +
      'again, and its projects can be added as usual.'
    exRemoveDialog.showModal()
  })

  exRemoveDialog.addEventListener('close', () => {
    const i = pendingExRemove
    pendingExRemove = null
    if (i === null || exRemoveDialog.returnValue !== 'remove') return
    EXCLUSIONS.splice(i, 1)
    renderExclusionRows()
    /* The confirmed row is gone from the DOM: keep focus inside the
       dialog on the next row's Remove, else the first remaining one,
       else Close */
    const nextFocus =
      exList.querySelector(`.ex-remove[data-idx="${i}"]`) ??
      exList.querySelector('.ex-remove') ??
      exCloseBtn
    nextFocus.focus()
  })

  /* ---------- Activity modal (LOG-001…006) ---------------------------------- */
  /* The §7.7 action-log window: user-visible entries only (LOG-005 —
     diagnostic records go to the tracing file log), newest first, with
     live filters by date, project, type and result (§7.7 UI row). An
     entry mirrors LOG-004's fields: time, operation type, object and
     result, plus the project and target path for file writes (the path
     is derived from PROJECTS via targetPathOf at render time). Bulk
     assignment ops log a single entry with the change count (LOG-004);
     the E_* codes repeat the sync summary demo. The filter choices are
     pure view state — the demo array never changes, and the selects keep
     their values across reopenings within the session. */

  const ACTIVITY = [
    /* Sync run of today, 14:32 — the one the sync bar counts as the last
       successful sync (LOG-007); run started → backup → write → finish */
    { minutesBack: 3, type: 'sync', action: 'Sync run started', project: null, result: 'ok' },
    {
      minutesBack: 2,
      type: 'backups',
      action: 'Backup created — AGENTS.md',
      project: 'EunomiaSync',
      file: true,
      result: 'ok',
    },
    {
      minutesBack: 1,
      type: 'sync',
      action: 'Target file written',
      project: 'EunomiaSync',
      file: true,
      result: 'ok',
    },
    {
      minutesBack: 0,
      type: 'sync',
      action: 'Sync run finished — status: success',
      project: null,
      result: 'ok',
    },
    /* Assignments: bulk ops log one entry with the change count (LOG-004) */
    {
      minutesBack: 45,
      type: 'assignments',
      action: 'Bulk assign — 12 assignments changed (“Naming conventions”)',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 200,
      type: 'assignments',
      action: 'Assigned variant “Conventional Commits” to “Commit messages”',
      project: 'docktree',
      result: 'ok',
    },
    /* Sync run of yesterday (finished 20:43) — partial: skips and
       failures do not stop it (D-09); per-project results carry the
       sync summary codes */
    {
      minutesBack: 1073,
      type: 'sync',
      action: 'File write skipped',
      code: 'E_MARKERS_INVALID',
      project: 'quotameter',
      file: true,
      result: 'skipped',
    },
    {
      minutesBack: 1072,
      type: 'sync',
      action: 'File write skipped',
      code: 'E_PROJECT_UNAVAILABLE',
      project: 'utils/deja-vu',
      file: true,
      result: 'skipped',
    },
    {
      minutesBack: 1071,
      type: 'sync',
      action: 'File write failed',
      code: 'E_FILE_LOCKED',
      project: 'sandbox-cf',
      file: true,
      result: 'error',
    },
    {
      minutesBack: 1070,
      type: 'sync',
      action: 'Target file written',
      project: 'task-id-mcp',
      file: true,
      result: 'ok',
    },
    {
      minutesBack: 1069,
      type: 'sync',
      action: 'Sync run finished — status: partial (2 skipped, 1 failed)',
      project: null,
      result: 'ok',
    },
    /* Library: instructions and variants, headings */
    {
      minutesBack: 1552,
      type: 'library',
      action: 'Added variant “Bug-first” to “Bug-driven development”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 2772,
      type: 'library',
      action: 'Edited instruction “Dependency audit”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 4532,
      type: 'library',
      action: 'Renamed heading “Git and branches” → “Git Workflow”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 6062,
      type: 'library',
      action: 'Moved heading “Communication” under “General”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 7127,
      type: 'library',
      action: 'Deleted variant “Old wording” of “Commit messages”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 8402,
      type: 'library',
      action: 'Restored instruction “Secrets handling”',
      project: null,
      result: 'ok',
    },
    {
      minutesBack: 8900,
      type: 'library',
      action: 'Hid instruction “Secrets handling”',
      project: null,
      result: 'ok',
    },
    { minutesBack: 24752, type: 'library', action: 'Created instruction “Dependency audit”', project: null, result: 'ok' },
    /* Projects: added (manual / scan), settings, archived, restored */
    { minutesBack: 9992, type: 'projects', action: 'Added project moli (manual)', project: 'moli', result: 'ok' },
    {
      minutesBack: 11227,
      type: 'projects',
      action: 'Added project sandbox-cf (found by scan)',
      project: 'sandbox-cf',
      result: 'ok',
    },
    {
      minutesBack: 12012,
      type: 'projects',
      action: 'Changed project settings — quotameter (target file CLAUDE.md)',
      project: 'quotameter',
      result: 'ok',
    },
    { minutesBack: 14332, type: 'projects', action: 'Restored project old-site', project: 'old-site', result: 'ok' },
    { minutesBack: 23360, type: 'projects', action: 'Archived project old-crm', project: 'old-crm', result: 'ok' },
  ]

  /* Human labels per entry type; the values double as the Type filter's
     option values */
  const ACT_TYPES = {
    projects: 'Projects',
    library: 'Library',
    assignments: 'Assignments',
    sync: 'Sync',
    backups: 'Backups',
  }

  const actDialog = document.getElementById('activity-dialog')
  const actTable = document.querySelector('#activity-dialog .act-table')
  const actRowsBody = document.getElementById('act-rows')
  const actEmpty = document.getElementById('act-empty')
  const actFilterDate = document.getElementById('act-filter-date')
  const actFilterProject = document.getElementById('act-filter-project')
  const actFilterType = document.getElementById('act-filter-type')
  const actFilterResult = document.getElementById('act-filter-result')

  /* Project filter options mirror the toolbar's demo projects */
  for (const name of projectNames) {
    const opt = document.createElement('option')
    opt.textContent = name
    actFilterProject.append(opt)
  }

  /* UTC+3 calendar day of a demo stamp, e.g. "19 Nov 2026" — the unit the
     Date filter compares in (the demo clock is fixed to UTC+3) */
  function actDay(minutesBack) {
    const d = new Date(HIST_NOW_MS - minutesBack * 60000 + 180 * 60000)
    return `${d.getUTCDate()} ${MONTHS_SHORT[d.getUTCMonth()]} ${d.getUTCFullYear()}`
  }

  const ACT_TODAY = actDay(0)

  /* The four filters combine; "today" compares calendar days, "7d" is a
     rolling window of 7 × 24 h. Newest first, like the history panel. */
  function renderActivity() {
    const date = actFilterDate.value // all | 7d | today
    const project = actFilterProject.value // all | demo project name
    const type = actFilterType.value // all | ACT_TYPES key
    const result = actFilterResult.value // all | ok | skipped | error

    const visible = ACTIVITY.filter((e) => {
      if (date === 'today' && actDay(e.minutesBack) !== ACT_TODAY) return false
      if (date === '7d' && e.minutesBack > 7 * 1440) return false
      if (project !== 'all' && e.project !== project) return false
      if (type !== 'all' && e.type !== type) return false
      if (result !== 'all' && e.result !== result) return false
      return true
    }).sort((a, b) => a.minutesBack - b.minutesBack)

    actRowsBody.innerHTML = visible
      .map((e) => {
        const path = e.file && e.project ? targetPathOf(e.project) : null
        const objectCell =
          (e.project ? escapeHtml(e.project) : '—') +
          (path ? `<code class="act-path">${escapeHtml(path)}</code>` : '')
        const code = e.code ? ` <span class="chip act-code">${escapeHtml(e.code)}</span>` : ''
        return (
          '<tr>' +
          `<td class="hist-when">${histStamp(e.minutesBack)}</td>` +
          `<td class="act-action"><span class="chip act-type">${ACT_TYPES[e.type]}</span>${escapeHtml(e.action)}${code}</td>` +
          `<td class="act-object">${objectCell}</td>` +
          `<td class="hist-op-cell"><span class="chip hist-op act-res act-res--${e.result}">${e.result}</span></td>` +
          '</tr>'
        )
      })
      .join('')

    actTable.hidden = visible.length === 0
    actEmpty.hidden = visible.length !== 0
  }

  /* Filters apply live on change; reopening keeps the last choice */
  for (const sel of [actFilterDate, actFilterProject, actFilterType, actFilterResult]) {
    sel.addEventListener('change', renderActivity)
  }

  function openActivity() {
    renderActivity()
    actDialog.showModal()
  }

  /* Entry point: the toolbar ghost button (left of Settings) */
  document.getElementById('activity-btn').addEventListener('click', openActivity)

  /* ---------- Help modal (HLP-001…004) -------------------------------------- */
  /* The FAQ window, opened by the toolbar "?" ghost button and F1
     (§11.3). Two-pane layout in the instruction-editor idiom: the
     article list and the search on the left, the selected article on
     the right. Article bodies are static demo documentation derived
     from the spec, rendered with the same escape-first mini renderer
     as the instruction preview (HLP-005: the FAQ is Markdown in
     English). Every markdown block is one source line — the renderer
     has no soft-wrap, so list items must not be hard-wrapped.
     Search (HLP-001) matches the title and the body case-insensitively;
     the corpus is the rendered plain text, cached once, so the list
     filter and the <mark> highlights in the body always agree. All
     highlighting splits DOM text (textContent / text nodes), so raw
     input is never injected as HTML. Non-matching articles are hidden;
     with zero matches the list shows the UI-012 empty state and the
     body keeps the last article. The selection persists across opens
     within the session. */

  const helpDialog = document.getElementById('help-dialog')
  const helpBtn = document.getElementById('help-btn')
  const helpList = document.getElementById('help-list')
  const helpEmpty = document.getElementById('help-empty')
  const helpSearch = document.getElementById('help-search')
  const helpSearchHint = document.getElementById('help-search-hint')
  const helpArticleTitle = document.getElementById('help-article-title')
  const helpArticle = document.getElementById('help-article')

  const HELP = [
    {
      title: 'Quick start',
      markdown: [
        'EunomiaSync stores instructions for AI agents in one local library and, on an explicit sync command, builds an `AGENTS.md` or `CLAUDE.md` file in each selected Git project — you write a rule once and assign it to the projects that need it.',
        '1. **Add a project** — pick a folder with `.git`, or run "Scan folder" to find repositories automatically (PRJ-001…007).',
        '2. **Create an instruction** — "New instruction" opens the editor: a name, the text of the `Main` variant, Markdown preview on the "Preview" tab (LIB-002).',
        '3. **Assign it** — click a cell in the assignment matrix; "Assign to projects…" covers many projects at once.',
        '4. **Run sync** — the pinned bottom panel builds and writes the target files. Nothing on disk changes until you run it (G5, OUT-011).',
        'A synced file ends with a `<!-- Created by EunomiaSync … -->` creation marker (D-23); a build identical to the file on disk is not rewritten (D-24).',
      ].join('\n'),
    },
    {
      title: 'Projects and scanning',
      markdown: [
        'A project is a local folder that is the root of a Git repository. A folder without `.git` is rejected with `E_NOT_A_REPO`; picking a subfolder offers the repository root (PRJ-001…004, D-08).',
        '- **Scan** walks the chosen folder up to `scan.max_depth` (6 by default, range 1–20, D-04) and skips `node_modules`, `vendor`, `.git`, `dist`, `build`, `target`, `.venv` by default — the list is editable in Settings (D-05).',
        '- A `.git` that is a **file** (submodule or linked worktree) is skipped with a reason; nested regular repositories become separate projects (D-06). Symlinks are not followed during a scan (D-07).',
        '- Results come in three groups: "New" (checked by default), "Already added", and "Skipped" with a reason. A folder without access is skipped and the scan continues (PRJ-007); closing the window cancels the scan (NFR-015).',
        '- A moved or deleted folder marks the project **unavailable** — "Locate…" or "Archive" (D-09); the sync skips it and finishes for the others.',
        '- Archiving (PRJ-013…018) is confirmed by typing the project name: files are untouched, assignments are kept, and the path lands in the "Excluded paths" list (D-10).',
      ].join('\n'),
    },
    {
      title: 'Instructions and variants',
      markdown: [
        'The library is one ordered structure of instructions and headings (`##`–`######`) shared by all projects. An instruction has a main name and one or more variants; exactly one variant is the main one.',
        '- The editor is a large two-pane modal: variants on the left, the text on the right, with "Edit" and "Preview" tabs (UI-003). Markdown warnings — for example an unclosed code fence — are shown but never block saving (D-15).',
        '- "Add variant" creates a copy of the main one or an empty variant. An empty variant outputs no text but counts as assigned and keeps its heading (D-13); a project can have only one variant of an instruction (D-02).',
        '- "Edit for this project" never changes a shared variant: it creates a project-local copy first and opens it (MAIN-007).',
        '- Deleting a variant that is assigned anywhere asks for a replacement per project; the last variant cannot be deleted (LIB-023…025).',
        '- "Hide" removes the instruction from builds but keeps its assignments (D-11); the "Show hidden" toggle and "Restore" bring it back.',
      ].join('\n'),
    },
    {
      title: 'Schemes A/B/C',
      markdown: [
        'Every project picks a write scheme and a target file (`AGENTS.md` or `CLAUDE.md`) in its settings; a new project copies the defaults (PRJ-012, DEF-005).',
        '#### Scheme A — full overwrite',
        'The target file is replaced entirely. Optional `<STEM>.local.md` and `<STEM>.project.md` reading lines can be included at the start or the end; local rules are read last and take priority (D-21).',
        '#### Scheme B — marked area',
        'The file must exist and contain exactly one pair of `<!-- EUNOMIASYNC:BEGIN -->` / `<!-- EUNOMIASYNC:END -->` markers, each on its own line (D-19). The app never creates them — use "Copy marker block" and paste the block by hand. Missing, repeated, or reversed markers give the `skipped` status with `E_MARKERS_INVALID`, and content outside the markers is kept byte for byte, including the BOM and line breaks.',
        '#### Scheme C — separate file',
        'The build goes to `<STEM>.project.md` in the project root, fully overwritten; the main file is never opened, and a ready snippet for it is shown in the project settings (D-20).',
        'Each written file ends with a `<!-- Created by EunomiaSync … -->` creation marker (D-23); content identical to the file on disk is not rewritten (D-24).',
      ].join('\n'),
    },
    {
      title: 'Sync and unsynced changes',
      markdown: [
        'Files change only on the explicit "Run sync" command, which covers all active projects (G5, OUT-011). Any edit is saved to the database right away; project files are not touched until the sync (MAIN-011).',
        '- The **dirty** flag is computed from the build hash — SHA-256 of the scheme, target path, extra-file flags and the build text without the creation marker (DB-020…021). It is recalculated about 150 ms after each change, at startup, and after a sync; the counter chip lists the dirty projects (OUT-013).',
        '- Changing the scheme or the file name makes a project dirty; the old file with the previous name is not deleted (DB-023).',
        '- A run goes: build from a snapshot → preflight → backups for all projects → atomic write → summary. A second start while running returns `E_SYNC_RUNNING` (MAIN-030, SYS-004).',
        '- The summary groups every project as "Written", "Unchanged", "Skipped", or "Failed", with the path and reason (OUT-009, MAIN-031). An unchanged build (D-24) counts as synced and creates no backup.',
        '- Externally edited target files are detected and shown — the app never merges them; the next sync overwrites the file as usual and makes a backup (OUT-014…015).',
      ].join('\n'),
    },
    {
      title: 'Backups',
      markdown: [
        'Before any file is written, backups of the existing target files are created and checked for all projects of the run; an error in even one backup cancels writing everywhere (OUT-006, BKP-005).',
        '- Copies live in the data directory: `backups/<project_id>__<slug>/<file>.<UTC time>.<hash8>.bak` (BKP-001).',
        '- A copy counts as created only after its size and SHA-256 are verified (BKP-010); if the source file matches the last backup, the existing copy is reused.',
        '- Up to `backup.retention` copies (10 by default, range 1–200) are kept per "project and file" pair; the oldest extras are removed after the run (BKP-003).',
        '- A failed backup stops the whole run before anything is written; copies made in that run are discarded, and there is no forced write without a backup (BKP-005).',
        '- The "Backups" tab in project settings lists date, file, and size (BKP-004). Restoring a file from a backup in the UI is planned for the next stage (P1) — for now, restore by hand from the data directory.',
      ].join('\n'),
    },
    {
      title: 'History and versions',
      markdown: [
        'The app records versions of instructions and variants. A version is created only on a real change of the snapshot — saving without edits adds nothing (LIB-026…027, D-16).',
        '- An instruction version stores the name and the active/hidden state; a variant version stores the name and the text. The main-variant flag and the library order are not part of snapshots.',
        '- Up to `history.max_revisions` versions (200 by default, range 10–1000) are kept per entity; the oldest ones are deleted (D-16).',
        '- The editor\'s "History" panel lists the versions with time, operation, and entity. A deleted variant can be brought back with "Restore variant".',
        '- Diff between versions and "Restore this version" are planned for the next stage (P1); a restore will create a new `revert` version, and old versions are never erased (LIB-028…029).',
        'Versions work next to the Activity log — the log records every user-visible action with time, type, object, and result (LOG-001…004).',
      ].join('\n'),
    },
    {
      title: 'Limits and common errors',
      markdown: [
        '- Variant text: Markdown, up to 262,144 characters; a line equal to a scheme B marker is rejected (`E_CONTENT_RESERVED`). CRLF is changed to LF on save (§6.9).',
        '- Instruction name: 1–200 characters, unique among visible instructions; variant name: 1–100 characters, unique within the instruction.',
        '- Scan depth 1–20 (6 by default); backup limit 1–200; versions per entity 10–1000; log 1,000–100,000 entries or 7–3,650 days (§6.5).',
        '**Common sync errors** (§10):',
        '- `E_MARKERS_INVALID` — scheme B markers are missing, repeated, or in the wrong order; paste the ready block with "Copy marker block".',
        '- `E_FILE_LOCKED` — the target file is locked by another process (Windows); close the program and retry, the backup is kept.',
        '- `E_NOT_A_REPO` — the folder has no `.git`; pick the repository root.',
        '- `E_SYNC_RUNNING` — a sync is already running; wait until it finishes.',
        'Every error is logged with its code, project, path, and OS reason; a sync is idempotent, so rerunning it after fixing the cause is safe (§10).',
      ].join('\n'),
    },
  ]

  /* Currently shown article; kept until the app reloads (session) */
  let helpSelected = 0

  /* The search corpus: rendered plain text, cached once per article */
  const helpScratch = document.createElement('div')
  for (const article of HELP) {
    helpScratch.innerHTML = renderMarkdown(article.markdown)
    article.plain = helpScratch.textContent
  }

  /* Append `text` into `into` (an element or a fragment), wrapping every
     case-insensitive occurrence of `lowerQuery` in <mark>. An empty
     query appends the text as is. HLP-001 highlight safety: the split
     runs on plain strings and produces only text nodes and <mark>, so
     no user input can become HTML. */
  function appendHighlighted(into, text, lowerQuery) {
    if (lowerQuery === '') {
      into.append(text)
      return
    }
    const lower = text.toLowerCase()
    let pos = 0
    for (let idx = lower.indexOf(lowerQuery); idx !== -1; idx = lower.indexOf(lowerQuery, pos)) {
      if (idx > pos) into.append(text.slice(pos, idx))
      const mark = document.createElement('mark')
      mark.textContent = text.slice(idx, idx + lowerQuery.length)
      into.append(mark)
      pos = idx + lowerQuery.length
    }
    if (pos < text.length) into.append(text.slice(pos))
  }

  /* Highlight inside the rendered article: a TreeWalker collects the
     text nodes first (already escapeHtml-escaped text — no tags can
     hide in them), then each node is replaced with mark-wrapped slices.
     The query is not highlighted across element boundaries (for example
     around bold) — such occurrences still match the filter. */
  function highlightArticle(root, lowerQuery) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
    const nodes = []
    while (walker.nextNode()) nodes.push(walker.currentNode)
    for (const node of nodes) {
      if (!node.nodeValue.toLowerCase().includes(lowerQuery)) continue
      const frag = document.createDocumentFragment()
      appendHighlighted(frag, node.nodeValue, lowerQuery)
      node.replaceWith(frag)
    }
  }

  function renderHelpList(matches, lowerQuery) {
    helpList.replaceChildren(
      ...HELP.map((article, i) => {
        const item = document.createElement('button')
        item.type = 'button'
        item.className = 'help-item'
        item.dataset.idx = String(i)
        item.hidden = !matches.includes(i)
        item.setAttribute('aria-pressed', String(i === helpSelected))
        const label = document.createElement('span')
        label.className = 'help-item-label'
        appendHighlighted(label, article.title, lowerQuery)
        item.append(label)
        return item
      }),
    )
  }

  function renderHelpBody(lowerQuery) {
    const article = HELP[helpSelected]
    helpArticleTitle.textContent = article.title
    /* renderMarkdown escapes the source first, then adds its own tags */
    helpArticle.innerHTML = renderMarkdown(article.markdown)
    if (lowerQuery !== '') highlightArticle(helpArticle, lowerQuery)
    helpArticle.scrollTop = 0
  }

  function applyHelpSearch() {
    const lowerQuery = helpSearch.value.trim().toLowerCase()

    /* HLP-001: match on the title or the rendered body text */
    const matches = []
    HELP.forEach((article, i) => {
      if (
        lowerQuery === '' ||
        article.title.toLowerCase().includes(lowerQuery) ||
        article.plain.toLowerCase().includes(lowerQuery)
      ) {
        matches.push(i)
      }
    })

    /* When the selected article is filtered out, fall back to the first
       match; with zero matches the body keeps the last article */
    if (lowerQuery !== '' && matches.length > 0 && !matches.includes(helpSelected)) {
      helpSelected = matches[0]
    }

    renderHelpList(matches, lowerQuery)
    renderHelpBody(lowerQuery)

    helpList.hidden = matches.length === 0
    helpEmpty.hidden = matches.length !== 0
    helpSearchHint.textContent =
      lowerQuery === ''
        ? `All ${HELP.length} articles`
        : `${matches.length} of ${HELP.length} articles match`
  }

  function selectHelpArticle(i) {
    if (i === helpSelected) return
    helpSelected = i
    for (const item of helpList.children) {
      item.setAttribute('aria-pressed', String(Number(item.dataset.idx) === i))
    }
    renderHelpBody(helpSearch.value.trim().toLowerCase())
  }

  helpList.addEventListener('click', (e) => {
    const item = e.target.closest('.help-item')
    if (item) selectHelpArticle(Number(item.dataset.idx))
  })

  helpSearch.addEventListener('input', applyHelpSearch)

  function openHelp() {
    /* A fresh open clears the search; the last article stays selected */
    helpSearch.value = ''
    applyHelpSearch()
    if (!helpDialog.open) helpDialog.showModal()
    const selected = helpList.querySelector('[aria-pressed="true"]')
    ;(selected ?? helpList.firstElementChild)?.focus()
  }

  /* Entry point 1: the toolbar ghost button */
  helpBtn.addEventListener('click', openHelp)

  /* Entry point 2 (§11.3): F1. Same guard idiom as Ctrl/Cmd + ,: ignored
     while the help dialog is already open; if another dialog is open,
     the native <dialog> stacking puts help on top. F1 is a dedicated
     key that typing cannot produce, so text entry in any other dialog's
     input never triggers it — no extra input-focused guard is needed. */
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'F1') return
    e.preventDefault()
    if (!helpDialog.open) openHelp()
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

  /* Row drag-and-drop between sections */
  initRowDrag()

  /* Toolbar icon buttons (Activity, Settings): glyphs from the ICONS
     idiom; the accessible names and tooltips live in the markup */
  document.getElementById('activity-btn').innerHTML = ICONS.activity
  document.getElementById('settings-btn').innerHTML = ICONS.settings

  /* Per-project edit icon in the header: opens the project settings modal,
     prefilled from the demo data of that column */
  for (const th of projectHeads) {
    const proj = th.querySelector('.proj-name')?.textContent.trim() ?? ''
    const line = th.querySelector('.proj-line')
    if (!line) continue
    const btn = document.createElement('button')
    btn.type = 'button'
    btn.className = 'cell-icon cell-icon--edit proj-edit'
    btn.innerHTML = ICONS.edit
    btn.title = `Edit project — ${proj}`
    btn.setAttribute('aria-label', `Edit project — ${proj}`)
    const p = PROJECTS.get(proj)
    if (p) p.editBtn = btn
    btn.addEventListener('click', () => {
      openProjectSettings(th.querySelector('.proj-name').textContent.trim())
    })
    line.append(btn)
  }

  /* Per-project custom instruction icons on every assignable cell */
  for (const td of document.querySelectorAll('.matrix tbody td.c-cell')) {
    refreshCellIcons(td)
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
