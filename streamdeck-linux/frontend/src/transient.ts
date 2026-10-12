// Process-memory UI state, persisted through the native shell when its window is hidden.
// This never goes into profile exports or plugin settings.
export type TextDraft = { text: string; source: string };
let textDrafts: Record<string, TextDraft> = {};
export function getTextDraft(key: string) {
  return textDrafts[key];
}
export function setTextDraft(key: string, value: TextDraft | undefined) {
  if (value) textDrafts[key] = value;
  else delete textDrafts[key];
  window.dispatchEvent(new Event("deckforge-draft-change"));
}
export function exportTextDrafts() {
  return textDrafts;
}
export function importTextDrafts(value: unknown) {
  if (value && typeof value === "object" && !Array.isArray(value))
    textDrafts = value as Record<string, TextDraft>;
}
export function hasTextDrafts(scope: string) {
  return Object.keys(textDrafts).some(
    (key) => key.startsWith(scope + ":") || key.startsWith(scope + "/"),
  );
}
export function clearTextDrafts(scope: string) {
  for (const key of Object.keys(textDrafts))
    if (key.startsWith(scope + ":") || key.startsWith(scope + "/"))
      delete textDrafts[key];
  window.dispatchEvent(new Event("deckforge-draft-change"));
}
let formDrafts: Record<string, any> = {};
export function getFormDraft(key: string) {
  return formDrafts[key];
}
export function setFormDraft(key: string, value: unknown) {
  formDrafts[key] = value;
  window.dispatchEvent(new Event("deckforge-draft-change"));
}
export function exportFormDrafts() {
  return formDrafts;
}
export function importFormDrafts(value: unknown) {
  if (value && typeof value === "object" && !Array.isArray(value))
    formDrafts = value as Record<string, any>;
}
