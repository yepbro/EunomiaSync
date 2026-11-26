/*
 * Help search highlighting (T-367, HLP-001): the case-insensitive
 * split of a text into plain and <mark>-wrapped segments. The demo
 * highlights via DOM text nodes (app.js appendHighlighted); the port
 * splits the string in the script and renders the segments in the
 * template — no v-html, no user input can become markup. The query is
 * not highlighted across element boundaries (the demo's rule — each
 * text is split on its own).
 */

/** One segment of a highlighted text: `mark` is true where the query
 * matched (the segment keeps its original casing). */
export type MarkSegment = { readonly text: string; readonly mark: boolean }

/** Split `text` around every case-insensitive occurrence of `query`.
 * An empty (or whitespace-only, to mirror the demo's trim) query
 * returns the text as one plain segment. */
export const splitMark = (text: string, query: string): readonly MarkSegment[] => {
  const needle = query.trim().toLowerCase()
  if (needle === '') return [{ text, mark: false }]
  const segments: MarkSegment[] = []
  const lower = text.toLowerCase()
  let pos = 0
  for (let idx = lower.indexOf(needle); idx !== -1; idx = lower.indexOf(needle, pos)) {
    if (idx > pos) segments.push({ text: text.slice(pos, idx), mark: false })
    segments.push({ text: text.slice(idx, idx + needle.length), mark: true })
    pos = idx + needle.length
  }
  if (pos < text.length) segments.push({ text: text.slice(pos), mark: false })
  return segments
}

/** Whether `text` contains the query (HLP-001: the match test for the
 * list filter — the same trim + lowercase the split applies). */
export const containsQuery = (text: string, query: string): boolean => {
  const needle = query.trim().toLowerCase()
  return needle === '' || text.toLowerCase().includes(needle)
}
