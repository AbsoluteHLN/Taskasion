// ProjectHLN UI System v2.3 - ESM runtime entry
// Industrial-Tactical-Vector Design System (curated from v2)

export const HLN_V23_THEMES = Object.freeze([
  "arknights",
  "endfield",
  "blacksteel",
  "abyss-aegir",
  "babel",
  "monster-siren",
])

export const HLN_V23_THEME_REGISTRY = Object.freeze([
  { id: "arknights",     label: "Arknights Tactical", swatch: "#131920", accent: "#00e5ff", colorScheme: "dark" },
  { id: "endfield",      label: "Endfield Industry",  swatch: "#121c22", accent: "#f6d000", colorScheme: "dark" },
  { id: "blacksteel",    label: "Blacksteel PMC",     swatch: "#141418", accent: "#ff9100", colorScheme: "dark" },
  { id: "abyss-aegir",   label: "Abyss Aegir",        swatch: "#091a28", accent: "#00ffd5", colorScheme: "dark" },
  { id: "babel",         label: "Babel Void",         swatch: "#120e20", accent: "#a855f7", colorScheme: "dark" },
  { id: "monster-siren", label: "Monster Siren",      swatch: "#0f0f12", accent: "#00ff66", colorScheme: "dark" },
])

export const HLN_V23_BG_PRESETS = Object.freeze([
  { id: "tactical-grid", label: "Tactical Vector Grid",  note: "precision coordinate scan" },
  { id: "holo-scan",     label: "Cyber Laser Scan",      note: "laser scanline sweep" },
  { id: "circuit-trace", label: "Circuit Trace Matrix",  note: "industrial trace grid" },
  { id: "prts-sonar",    label: "PRTS Sonar Sweep",      note: "circular tactical radar" },
  { id: "hazard-hatch",  label: "Hazard Hatch Field",    note: "industrial diagonal stripes" },
  { id: "hex-field",     label: "Hex Field Lattice",   note: "tessellated hex grid" },
  { id: "radar-cross",   label: "Radar Crosshair",     note: "rotating sweep + cross" },
  { id: "data-rain",     label: "Data Rain Columns",   note: "falling vector stream" },
  { id: "blueprint",     label: "Blueprint Draft Grid",note: "fine drafting matrix" },
  { id: "data-lattice",      label: "Data Lattice Matrix", note: "falling glyph grid + scan" },
  { id: "blueprint-schematic",label: "Blueprint Schematic", note: "drafting grid + dimension lines" },
  { id: "quiet",         label: "Static Quiet",          note: "zero background motion" },
])

export const HLN_V23_FONTS = Object.freeze([
  "display",
  "technical",
  "grotesque",
  "cyber",
  "berlin",
  "condensed",
])

export function isHlnV23Theme(theme) {
  return HLN_V23_THEMES.includes(theme)
}

export function applyHlnV23Theme(theme, target = typeof document !== "undefined" ? document.body : null) {
  if (!target || !isHlnV23Theme(theme)) return false
  target.dataset.hlnTheme = theme
  target.dataset.theme = theme
  return true
}

export function currentHlnV23Theme(target = typeof document !== "undefined" ? document.body : null) {
  return target?.dataset?.hlnTheme ?? "arknights"
}

export function isHlnV23Font(font) {
  return HLN_V23_FONTS.includes(font)
}

export function applyHlnV23Font(font, target = typeof document !== "undefined" ? document.body : null) {
  if (!target || !isHlnV23Font(font)) return false
  target.dataset.hlnFont = font
  return true
}

export function currentHlnV23Font(target = typeof document !== "undefined" ? document.body : null) {
  return target?.dataset?.hlnFont ?? "display"
}

export function playHlnV23Motion(scope, preset = "panel", state = "enter", variant) {
  if (!scope || typeof window === "undefined") return
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return
  const selector = "[data-hln-motion=" + JSON.stringify(preset) + "]"
  const elements = scope.matches?.(selector) ? [scope] : [...scope.querySelectorAll(selector)]
  elements.forEach((element, index) => {
    element.style.setProperty("--hln-ui-motion-index", String(index))
    delete element.dataset.hlnMotionState
    delete element.dataset.hlnMotionVariant
    void element.offsetWidth
    element.dataset.hlnMotionState = state
    if (variant) element.dataset.hlnMotionVariant = variant
    element.addEventListener("animationend", () => {
      delete element.dataset.hlnMotionState
      delete element.dataset.hlnMotionVariant
    }, { once: true })
  })
}
