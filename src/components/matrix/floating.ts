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
 * Close-on-dismiss for a mounted floating box: outside click and Escape
 * call `close` (the skeleton's minimum usability; scroll-close and the
 * real behavior are T-184/T-355's). `ignoreSelector` names the trigger
 * so the toggle click that opened the box does not immediately close
 * it — the demo's own hit-test idiom (app.js checks the target against
 * `.col-menu-btn` / `.cell-icon--eye`), a target hit-test, not a
 * structural traversal.
 */
export function useDismiss(
  target: Ref<HTMLElement | null>,
  close: () => void,
  ignoreSelector?: string,
): void {
  const onDocumentClick = (event: MouseEvent): void => {
    const box = target.value
    if (box === null) return
    const hit = event.target instanceof Element ? event.target : null
    if (ignoreSelector !== undefined && hit?.closest(ignoreSelector) !== null) return
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
