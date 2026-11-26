/*
 * Floating-layer helpers (T-364): the positioning and dismissal math
 * the column menu and the variant popover share — ported from the
 * demo's openColumnMenu / openVariantsPopover (docs/prototype/app.js
 * 1101–1116 and 1274–1286). CONTAINER-AGNOSTIC BY CONTRACT: the
 * positioning computes only from the component's own element and an
 * anchor handed in as data (the trigger's bounding-rect numbers) — no
 * DOM-order traversal — so T-169 can re-host the components in a
 * virtualized container. Each component keeps its OWN demo formula:
 * the menu right-aligns under the trigger (gap 4, app.js 1110–1112),
 * the popover left-aligns (gap 6, app.js 1280–1282).
 */
import { onBeforeUnmount, onMounted, type Ref } from 'vue'

/**
 * The trigger's bounding-rect numbers a floating box aligns to. Plain
 * data (the caller reads its own trigger's rect), so the box can be
 * re-hosted anywhere.
 */
export type Anchor = {
  readonly left: number
  readonly right: number
  readonly top: number
  readonly bottom: number
}

/** The viewport margin the demo keeps around a clamped box. */
const MARGIN = 8

/**
 * Where a floating box of the given element's size goes: below the
 * trigger — its right edge on the trigger's right (`align: 'end'`, the
 * column menu) or its left edge on the trigger's left
 * (`align: 'start'`, the variant popover) — clamped to the viewport,
 * flipped above the trigger's TOP when it would overflow the bottom
 * (the demo's formulas, verbatim per component).
 */
export function placeFloating(
  anchor: Anchor,
  el: HTMLElement,
  options: { readonly align: 'start' | 'end'; readonly gap: number },
): { left: number; top: number } {
  const w = el.offsetWidth
  const h = el.offsetHeight
  const x = Math.min(
    Math.max(MARGIN, options.align === 'end' ? anchor.right - w : anchor.left),
    Math.max(MARGIN, window.innerWidth - w - MARGIN),
  )
  let y = anchor.bottom + options.gap
  if (y + h > window.innerHeight - MARGIN) y = Math.max(MARGIN, anchor.top - h - options.gap)
  return { left: Math.round(x), top: Math.round(y) }
}

/**
 * The kinds of floating layer the matrix family opens. At most ONE
 * layer of each kind may be open (review H2; MAIN-032, MAIN-033 — the
 * demo has one #variants-popover and one #column-menu). The
 * single-owner re-host is card T-368's; until then the invariant lives
 * here, structurally: a layer claims its kind's slot when it mounts
 * (each mounts only while open) and the claim closes the previous
 * holder.
 */
export type FloatingLayerKind = 'popover' | 'menu'

/** The close callback of the layer currently holding each kind's slot. */
const openLayers = new Map<FloatingLayerKind, () => void>()

/**
 * The exclusive-slot invariant for one mounted floating layer:
 * claiming the kind's single slot closes the layer that holds it;
 * unmounting releases the slot — only if this instance still holds it,
 * so a layer closed by a successor does not free the successor's slot.
 */
export function useExclusiveLayer(kind: FloatingLayerKind, close: () => void): void {
  const previous = openLayers.get(kind)
  openLayers.set(kind, close)
  if (previous !== undefined) previous()
  onBeforeUnmount(() => {
    if (openLayers.get(kind) === close) openLayers.delete(kind)
  })
}

/**
 * Close-on-dismiss for a mounted floating box: outside click and Escape
 * call `close` (the skeleton's minimum usability; scroll-close and the
 * real behavior are T-184/T-355's). `ignore` is the layer's OWN trigger
 * element (this eye, this "⋮") — the toggle click that opened the box
 * must not immediately close it. It is an element REFERENCE, not the
 * demo's class hit-test (app.js checks the target against
 * `.col-menu-btn` / `.cell-icon--eye`): a class matches ANY instance's
 * trigger, which left two layers of the same kind open at once
 * (review H2).
 */
export function useDismiss(
  target: Ref<HTMLElement | null>,
  close: () => void,
  ignore?: Ref<Element | null | undefined>,
): void {
  const onDocumentClick = (event: MouseEvent): void => {
    const box = target.value
    if (box === null) return
    const hit = event.target instanceof Element ? event.target : null
    const own = ignore?.value
    if (own !== null && own !== undefined && (hit === own || own.contains(hit))) return
    if (hit === null || !box.contains(hit)) close()
  }
  const onKeydown = (event: KeyboardEvent): void => {
    if (event.key === 'Escape') close()
  }
  onMounted(() => {
    document.addEventListener('click', onDocumentClick)
    document.addEventListener('keydown', onKeydown)
  })
  onBeforeUnmount(() => {
    document.removeEventListener('click', onDocumentClick)
    document.removeEventListener('keydown', onKeydown)
  })
}

/** The anchor data for an opening floating box: the trigger's rect. */
export function anchorOf(trigger: Element): Anchor {
  const r = trigger.getBoundingClientRect()
  return { left: r.left, right: r.right, top: r.top, bottom: r.bottom }
}
